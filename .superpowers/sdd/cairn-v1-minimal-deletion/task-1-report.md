# Task 1 report

RESULT: Visible `cairn` CLI now exposes only `setup`; hidden `hook` and `mcp` remain callable. MCP exposes `cairn_context`, `cairn_search`, `cairn_remember`, `cairn_session`, and `cairn_handoff` only. `cairn_task`, `bind_task`, and task schema arguments are removed.

CHANGED:

- Removed public CLI dispatchers, aliases, renderers, integration and update entry points from `crates/cairn/src/main.rs`.
- Deleted obsolete CLI-only rendering and daemon proxy code.
- Reduced MCP schema and handler surface to five tools; removed task scopes from search and remember schemas.
- Updated public help snapshot and behavior tests.

VALIDATION:

- RED: `default_help_matches_golden` failed against former help snapshot.
- GREEN: `PATH=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH rustup run 1.97.1 cargo test -p cairn --bin cairn` — 23 passed.
- `PATH=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH rustup run 1.97.1 cargo clippy -p cairn --bin cairn -- -D warnings` — passed.
- Live `cairn --help` lists only `setup`; `git diff --check` passed.

UNCERTAINTY:

- `Request::SessionStart` in current core still requires `task_id`; MCP supplies `task_id: None` at `crates/cairn/src/mcp.rs:620` without accepting any task input. Remove that compatibility initializer when Task 2 removes the wire field.

NEXT: Integrate with Task-domain wire deletion, then rerun workspace verification.

## Review fix 1/5

RESULT: Disabled Clap's `help` subcommand, restored global `--json` success and error envelopes for `setup`, removed stale six-tool/task test surfaces, and deleted orphan CLI integration/update modules.

CHANGED:

- `crates/cairn/src/main.rs`: `disable_help_subcommand = true`; strict visible-command test; `setup` text/JSON rendering seam and coverage.
- `crates/cairn/src/integrate.rs`, `crates/cairn/src/update.rs`: deleted orphan modules.
- `tests/src/lib.rs`: sandbox bootstrap now calls `setup` rather than deleted `init` alias.
- `tests/tests/manual_mcp_mode.rs`, `tests/tests/generic_mcp.rs`: retained five-tool manual MCP coverage without task binding.
- `tests/tests/cli_rendering.rs`, `tests/tests/mcp_backward_compatibility.rs`: deleted obsolete broad CLI and six-tool compatibility fixtures.

VALIDATION:

- RED: `PATH=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH rustup run 1.97.1 cargo test -p cairn setup_renders_stable_text_and_json_envelopes -- --nocapture` failed with missing rendering seam.
- GREEN: `PATH=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH rustup run 1.97.1 cargo test -p cairn --bin cairn` — 24 passed.
- `PATH=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH rustup run 1.97.1 cargo test -p cairn-e2e --test manual_mcp_mode --test generic_mcp` — 4 passed.
- `PATH=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH rustup run 1.97.1 cargo test --workspace --all-targets --no-run` — passed.
- `PATH=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin:$PATH rustup run 1.97.1 cargo clippy -p cairn --bin cairn -- -D warnings` — passed.
