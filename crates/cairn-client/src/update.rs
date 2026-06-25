use anyhow::Context;
use self_update::cargo_crate_version;

const REPO_OWNER: &str = "Vellixia";
const REPO_NAME: &str = "cairn";

pub fn run(check_only: bool) -> anyhow::Result<()> {
    let current = cargo_crate_version!();

    if check_only {
        let release = self_update::backends::github::Update::configure()
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .bin_name("cairn")
            .current_version(current)
            .build()
            .context("building updater")?
            .get_latest_release()
            .context("fetching latest release")?;

        let latest = release.version.trim_start_matches('v');
        if self_update::version::bump_is_greater(current, latest).unwrap_or(false) {
            println!("Update available: {current} -> {latest}");
            println!("Run `cairn upgrade` to install.");
        } else {
            println!("Already up to date ({current}).");
        }
        return Ok(());
    }

    let status = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name("cairn")
        .current_version(current)
        .no_confirm(true)
        .show_download_progress(true)
        .show_output(false)
        .build()
        .context("building updater")?
        .update()
        .context("downloading and installing update")?;

    match status {
        self_update::Status::UpToDate(v) => {
            println!("Already at {v} - nothing to do.");
        }
        self_update::Status::Updated(v) => {
            println!("Updated to {v}.");
            println!("Restart any running `cairn mcp` processes to pick up the new binary.");
        }
    }
    Ok(())
}
