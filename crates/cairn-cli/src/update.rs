//! `cairn-cli update` — replace the running binary with the latest GitHub release.
//!
//! Backed by the well-tested `self_update` crate (ureq + rustls backend), which picks the release
//! asset matching this platform's target triple (the release workflow names them
//! `cairn-<target>.tar.gz`/`.zip`), verifies and extracts it, and atomically swaps the running
//! executable. The combined archive contains both `cairn` and `cairn-cli`; this updates `cairn-cli`.

use anyhow::{Context, Result};

pub fn run() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");

    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner("Vellixia")
        .repo_name("Cairn")
        .bin_name("cairn-cli")
        .current_version(current)
        .show_download_progress(true);

    // A token lifts GitHub's unauthenticated API rate limit (and is needed for private mirrors).
    if let Ok(token) =
        std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("CAIRN_GITHUB_TOKEN"))
    {
        if !token.is_empty() {
            builder.auth_token(&token);
        }
    }

    let status = builder
        .build()
        .context("preparing the updater")?
        .update()
        .context(
            "checking for or applying the update \
             (if you're rate-limited, set GITHUB_TOKEN and retry)",
        )?;

    if status.updated() {
        println!("cairn-cli updated {current} → {}", status.version());
        println!("Re-run your command to use the new version.");
    } else {
        println!("cairn-cli is already up to date ({current}).");
    }
    Ok(())
}
