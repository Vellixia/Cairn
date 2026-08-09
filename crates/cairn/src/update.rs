//! `cairn update` — find a newer release and install it.
//!
//! Two rules shape this. Nothing is installed that has not matched the digest
//! the release publishes, because an updater that silently accepts a corrupted
//! or substituted archive is worse than no updater. And `cairn` and `cairnd`
//! are replaced together: they speak a private protocol to each other, so a
//! half-applied update leaves a machine where the CLI cannot talk to its own
//! daemon.

use cairn_core::release::{self, Release};
use cairn_core::wire::WireError;
use std::io::Read;
use std::path::{Path, PathBuf};

const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// What an update attempt found, so the caller can render it.
pub struct Outcome {
    pub current: String,
    pub latest: Option<Release>,
    pub update_available: bool,
    /// Set when binaries were actually replaced.
    pub installed: bool,
    /// Where they were replaced, for the report.
    pub installed_to: Vec<PathBuf>,
}

fn http() -> Result<reqwest::Client, WireError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        // GitHub rejects requests without one.
        .user_agent(concat!("cairn/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| WireError::invalid(format!("could not build an HTTP client: {e}")))
}

/// Ask what the newest release is.
pub async fn check() -> Result<Outcome, WireError> {
    let client = http()?;
    let body = client
        .get(release::RELEASES_API)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| WireError::invalid(format!("could not reach GitHub: {e}")))?
        .error_for_status()
        .map_err(|e| WireError::invalid(format!("GitHub refused the request: {e}")))?
        .text()
        .await
        .map_err(|e| WireError::invalid(format!("could not read the response: {e}")))?;

    let latest = release::pick_release(&body, CURRENT);
    let update_available = latest
        .as_ref()
        .is_some_and(|r| release::update_available(CURRENT, &r.version));

    Ok(Outcome {
        current: CURRENT.to_string(),
        latest,
        update_available,
        installed: false,
        installed_to: Vec::new(),
    })
}

/// Check, and install if there is something newer.
pub async fn apply() -> Result<Outcome, WireError> {
    let mut outcome = check().await?;
    if !outcome.update_available {
        return Ok(outcome);
    }
    let release = outcome
        .latest
        .clone()
        .expect("update_available implies one");

    let target = release::target_triple().ok_or_else(|| {
        WireError::invalid(format!(
            "no release is published for {}-{}; install from {}",
            std::env::consts::ARCH,
            std::env::consts::OS,
            release::RELEASES_PAGE
        ))
    })?;

    // Refuse early rather than after a download, so a read-only install
    // directory costs nobody a transfer.
    let targets = install_targets()?;
    for path in &targets {
        writable(path)?;
    }

    let client = http()?;
    let archive_name = release::archive_name(&release.version, target);
    let archive = download(&client, &release::asset_url(&release.tag, &archive_name)).await?;
    let sums = download(&client, &release::asset_url(&release.tag, "SHA256SUMS")).await?;
    let sums = String::from_utf8_lossy(&sums);

    let expected = release::expected_digest(&sums, &archive_name)
        .ok_or_else(|| WireError::invalid(format!("{archive_name} is not listed in SHA256SUMS")))?;
    let actual = sha256(&archive);
    if actual != expected {
        return Err(WireError::invalid(format!(
            "{archive_name} does not match its published digest; refusing to install \
             (expected {expected}, got {actual})"
        )));
    }

    let staged = unpack(&archive, &release.version, target)?;
    for path in &targets {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| WireError::invalid("could not read an install path"))?;
        let new = staged.path().join(name);
        replace(&new, path)?;
    }

    outcome.installed = true;
    outcome.installed_to = targets;
    Ok(outcome)
}

/// The `cairn` and `cairnd` this install owns.
///
/// Taken from the running executable rather than `PATH`, so an update applies
/// to the copy actually in use when several are installed.
fn install_targets() -> Result<Vec<PathBuf>, WireError> {
    let exe = std::env::current_exe()
        .map_err(|e| WireError::invalid(format!("could not locate this binary: {e}")))?;
    let dir = exe
        .parent()
        .ok_or_else(|| WireError::invalid("this binary has no parent directory"))?;
    Ok(vec![dir.join("cairn"), dir.join("cairnd")])
}

/// Whether this process may replace `path`.
fn writable(path: &Path) -> Result<(), WireError> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let probe = dir.join(format!(".cairn-update-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(WireError::invalid(format!(
            "cannot write to {}: {e}. Re-run with the rights to replace it, \
             or install manually from {}",
            dir.display(),
            release::RELEASES_PAGE
        ))),
    }
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, WireError> {
    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|e| WireError::invalid(format!("could not download {url}: {e}")))?
        .error_for_status()
        .map_err(|e| WireError::invalid(format!("could not download {url}: {e}")))?
        .bytes()
        .await
        .map_err(|e| WireError::invalid(format!("could not read {url}: {e}")))?;
    Ok(bytes.to_vec())
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Extract the two binaries into a temporary directory of their own.
///
/// Only the expected names are taken, so a malformed archive cannot write
/// anywhere it likes.
fn unpack(archive: &[u8], version: &str, target: &str) -> Result<tempfile::TempDir, WireError> {
    let dir = tempfile::tempdir()
        .map_err(|e| WireError::invalid(format!("could not make a staging directory: {e}")))?;
    let root = format!("cairn-v{version}-{target}");

    let decoder = flate2::read::GzDecoder::new(archive);
    let mut tar = tar::Archive::new(decoder);
    let entries = tar
        .entries()
        .map_err(|e| WireError::invalid(format!("could not read the archive: {e}")))?;

    let mut found = 0;
    for entry in entries {
        let mut entry =
            entry.map_err(|e| WireError::invalid(format!("could not read the archive: {e}")))?;
        let path = entry
            .path()
            .map_err(|e| WireError::invalid(format!("could not read the archive: {e}")))?
            .into_owned();

        let wanted = matches!(
            path.strip_prefix(&root).ok().and_then(|p| p.to_str()),
            Some("cairn") | Some("cairnd")
        );
        if !wanted {
            continue;
        }
        let name = path.file_name().expect("matched above").to_owned();
        let out = dir.path().join(&name);

        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| WireError::invalid(format!("could not read {name:?}: {e}")))?;
        std::fs::write(&out, &bytes)
            .map_err(|e| WireError::invalid(format!("could not stage {name:?}: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o755));
        }
        found += 1;
    }

    if found != 2 {
        return Err(WireError::invalid(format!(
            "the archive did not contain both cairn and cairnd (found {found})"
        )));
    }
    Ok(dir)
}

/// Put `new` where `target` is.
///
/// Renaming over a running binary is safe on Unix — the running process keeps
/// its open inode — and is atomic, so a reader never sees a half-written file.
/// The staging copy is made in the destination directory because `rename` does
/// not cross filesystems.
fn replace(new: &Path, target: &Path) -> Result<(), WireError> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let staged = dir.join(format!(
        ".{}.cairn-update-{}",
        target.file_name().and_then(|n| n.to_str()).unwrap_or("bin"),
        std::process::id()
    ));
    std::fs::copy(new, &staged)
        .map_err(|e| WireError::invalid(format!("could not stage {}: {e}", target.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&staged, target).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        WireError::invalid(format!("could not replace {}: {e}", target.display()))
    })
}
