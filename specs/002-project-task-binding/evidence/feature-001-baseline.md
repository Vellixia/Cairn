# Feature 001 regression baseline

- Execution date: 2026-07-22 (Asia/Jakarta)
- Tested checkout commit: `fb204b07d6c8ee4e1f82fce262feb311086df551`
- Working tree before execution: Feature 002 specification artifacts modified; no runtime source modified
- OS: macOS Darwin 25.5.0, arm64
- Rust: `rustc 1.90.0-nightly (abf50ae2e 2025-09-16)`
- Cargo: `cargo 1.90.0-nightly (840b83a10 2025-07-30)`
- External network requirement: none

## Executed commands

| Command | Result |
|---|---|
| `cargo test --workspace --all-targets` | PASS; all non-ignored workspace targets passed, with the explicit performance test remaining ignored as designed |
| `cargo test -p cairn-daemon --test us2_agent_sim` | PASS; 1/1 |
| `cargo test -p cairn-daemon --test us3_tracking` | PASS; 10/10 |
| `cargo test -p cairn-daemon --test us3_events` | PASS; 2/2 |

The workspace run included repository, worktree, snapshot, session, event, replay,
recovery, privacy, IPC, CLI, domain, Git, and storage suites. No test was weakened,
deleted, or newly ignored for this baseline.

## Decision

The Feature 001 regression baseline is green. Feature 002 foundational runtime work
may proceed only after the separate frozen-producer fixture and schema checks also
pass.
