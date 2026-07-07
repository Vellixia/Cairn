//! `cairn uninstall` — remove all Cairn-managed entries and state.
//!
//! 1. **Remove agent entries** — same as `cairn reset`: strips cairn-owned
//!    hooks, MCP entries, and managed blocks from every detected agent config.
//! 2. **Delete `~/.cairn/`** — config.toml, spool.jsonl, hook logs, session
//!    buffers, everything.
//!
//! Safe to run multiple times (idempotent): second pass finds nothing.

use anyhow::Result;

pub fn run(dry_run: bool) -> Result<()> {
    // Step 1: agent config cleanup (reuse reset logic).
    crate::reset::run(dry_run)?;

    // Step 2: delete ~/.cairn/ directory.
    if let Some(cairn_home) = crate::paths::cairn_home() {
        if cairn_home.exists() {
            if dry_run {
                println!(
                    "Would remove Cairn state directory: {}",
                    cairn_home.display()
                );
            } else {
                std::fs::remove_dir_all(&cairn_home)?;
                println!("Removed state directory: {}", cairn_home.display());
            }
        }
    }

    if !dry_run {
        println!("\nCairn has been fully removed from this machine.");
    }
    Ok(())
}
