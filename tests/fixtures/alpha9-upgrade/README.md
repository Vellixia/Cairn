# Alpha.7 upgrade fixture

`alpha9_upgrade.rs` builds the pinned alpha.7 schema (v12) with the retained
release migrations, leaves its representative pending rows in SQLite WAL, and
runs the current installed setup/daemon path. The temporary fixture is kept
runtime-generated so its WAL remains valid for the platform SQLite library.
