//! T021: IPC server — UDS (0700 dir) on Unix; Windows named pipe with an
//! explicit DACL restricted to the current user SID plus SYSTEM/Admins
//! (analysis I1 — default pipe permissions are never relied upon).
//! JSON-lines framing; per-user singleton via bind race.
//!
//! Resume tokens transit only this authenticated local IPC channel.

#[cfg(unix)]
use anyhow::bail;
use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::watch;

use crate::router;
use crate::state::AppState;

async fn handle_conn<S>(state: AppState, stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut lines = BufReader::new(read_half).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<cairn_protocol::Request>(&line) {
            Ok(req) => router::dispatch(&state, req).await,
            Err(e) => cairn_protocol::Response::err(
                "",
                cairn_protocol::ErrorCode::Usage,
                format!("malformed request: {e}"),
            ),
        };
        let mut payload = serde_json::to_string(&response).expect("serializable response");
        payload.push('\n');
        if write_half.write_all(payload.as_bytes()).await.is_err() {
            break;
        }
    }
}

#[cfg(unix)]
pub async fn serve(state: AppState, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::{UnixListener, UnixStream};

    let path = state.inner.config.socket_path.clone();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    // Singleton bind race: if another daemon answers, exit; else remove stale.
    if path.exists() {
        if UnixStream::connect(&path).await.is_ok() {
            bail!("another cairnd is already serving {}", path.display());
        }
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!(socket = %path.display(), "ipc listening");

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(handle_conn(state.clone(), stream));
            }
        }
    }
    let _ = std::fs::remove_file(&path);
    Ok(())
}

#[cfg(windows)]
pub async fn serve(state: AppState, mut shutdown: watch::Receiver<bool>) -> Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let name = format!(r"\\.\pipe\{}", state.inner.config.pipe_name);
    let sd = windows_security::pipe_security_descriptor()?;

    // First instance claims the singleton; failure = already running.
    let mut server = unsafe {
        ServerOptions::new()
            .first_pipe_instance(true)
            .create_with_security_attributes_raw(&name, sd.attributes_ptr())
    }
    .map_err(|e| anyhow::anyhow!("another cairnd may already own {name}: {e}"))?;
    tracing::info!(pipe = %name, "ipc listening");

    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            connected = server.connect() => {
                connected?;
                // Hand the connected instance to a task; create the next one.
                let next = unsafe {
                    ServerOptions::new()
                        .create_with_security_attributes_raw(&name, sd.attributes_ptr())
                }?;
                let stream = std::mem::replace(&mut server, next);
                tokio::spawn(handle_conn(state.clone(), stream));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
mod windows_security {
    //! Explicit DACL: current user SID + SYSTEM + Builtin Administrators,
    //! built from an SDDL string (analysis I1).

    use anyhow::{bail, Result};
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// Owns the security descriptor memory for the server's lifetime.
    pub struct PipeSecurity {
        descriptor: *mut core::ffi::c_void,
        attributes: Box<SECURITY_ATTRIBUTES>,
    }

    unsafe impl Send for PipeSecurity {}
    unsafe impl Sync for PipeSecurity {}

    impl PipeSecurity {
        pub fn attributes_ptr(&self) -> *mut core::ffi::c_void {
            &*self.attributes as *const SECURITY_ATTRIBUTES as *mut core::ffi::c_void
        }
    }

    impl Drop for PipeSecurity {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.descriptor);
            }
        }
    }

    fn current_user_sid() -> Result<String> {
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                bail!(
                    "OpenProcessToken failed: {}",
                    std::io::Error::last_os_error()
                );
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
            let mut buf = vec![0u8; len as usize];
            let ok = GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr() as *mut core::ffi::c_void,
                len,
                &mut len,
            );
            CloseHandle(token);
            if ok == 0 {
                bail!(
                    "GetTokenInformation failed: {}",
                    std::io::Error::last_os_error()
                );
            }
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut sid_str: *mut u16 = std::ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut sid_str) == 0 {
                bail!(
                    "ConvertSidToStringSidW failed: {}",
                    std::io::Error::last_os_error()
                );
            }
            let mut n = 0;
            while *sid_str.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(sid_str, n));
            LocalFree(sid_str as *mut core::ffi::c_void);
            Ok(s)
        }
    }

    pub fn pipe_security_descriptor() -> Result<PipeSecurity> {
        let sid = current_user_sid()?;
        // Generic-all for: current user, LocalSystem, Builtin Administrators.
        // Protected DACL (P) — no inherited ACEs, no Everyone access.
        let sddl = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)(A;;GA;;;BA)");
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let mut descriptor: *mut core::ffi::c_void = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor as *mut _
                    as *mut windows_sys::Win32::Security::PSECURITY_DESCRIPTOR,
                std::ptr::null_mut(),
            ) == 0
            {
                bail!(
                    "ConvertStringSecurityDescriptorToSecurityDescriptorW failed: {}",
                    std::io::Error::last_os_error()
                );
            }
            let attributes = Box::new(SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            });
            Ok(PipeSecurity {
                descriptor,
                attributes,
            })
        }
    }
}
