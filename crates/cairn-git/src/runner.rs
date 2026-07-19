//! T018: async Git subprocess runner with NUL-safe capture and timeout guard.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::error::GitError;

const GIT_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `git` subcommands inside a fixed working directory.
#[derive(Debug, Clone)]
pub struct GitRunner {
    dir: PathBuf,
}

impl GitRunner {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Run git with `args`; return raw stdout bytes (NUL-safe for `-z` modes).
    pub async fn run(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let mut child = Command::new("git")
            .arg("-c")
            .arg("core.quotepath=false")
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    GitError::GitUnavailable(e.to_string())
                } else {
                    GitError::CommandFailed(e.to_string())
                }
            })?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut out_pipe = child.stdout.take().expect("piped stdout");
        let mut err_pipe = child.stderr.take().expect("piped stderr");

        let fut = async {
            let (a, b, status) = tokio::join!(
                out_pipe.read_to_end(&mut stdout),
                err_pipe.read_to_end(&mut stderr),
                child.wait(),
            );
            a.map_err(GitError::Io)?;
            b.map_err(GitError::Io)?;
            status.map_err(GitError::Io)
        };

        let status = tokio::time::timeout(GIT_TIMEOUT, fut)
            .await
            .map_err(|_| GitError::CommandFailed(format!("git {args:?} timed out")))??;

        if !status.success() {
            let msg = String::from_utf8_lossy(&stderr).trim().to_string();
            if msg.contains("not a git repository") {
                return Err(GitError::NotARepository(self.dir.display().to_string()));
            }
            return Err(GitError::CommandFailed(format!("git {args:?}: {msg}")));
        }
        Ok(stdout)
    }

    /// Run and decode stdout as UTF-8 (lossy), trimmed.
    pub async fn run_text(&self, args: &[&str]) -> Result<String, GitError> {
        let out = self.run(args).await?;
        Ok(String::from_utf8_lossy(&out).trim_end().to_string())
    }
}
