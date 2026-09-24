# V1 deletion baseline

Frozen comparison commit: `37417192f316cb495601edf44d5bddfa967b0e5c`.

Run `bash scripts/v1-baseline.sh` for reproducible source and surface inventory. The script reads the frozen tree without changing it and compares it with the current worktree. Production includes Rust under `crates/*/src` with guarded test items excluded, plus TypeScript/TSX under `web/{app,components,hooks,lib}`. Tests, fixtures, generated output, dependencies, and deleted narrative specifications earn no production-reduction credit.

Measured at `e9e005df60e1a3a196f290c710cb2b32124049d3`:

| Production source | Frozen | V1 | Delta |
|---|---:|---:|---:|
| Rust | 82,783 | 46,550 | -36,233 |
| Web | 9,256 | 7,851 | -1,405 |
| Total | 92,039 | 54,401 | -37,638 |

Measured surfaces: three CLI enum entries (`setup` plus hidden `hook` and `mcp`), five MCP tools, nine web pages, and 61 explicit HTTP/typed web-operation declarations. Applied migrations remain immutable history, so raw migration table counts are not runtime edge-schema counts.

Current architecture and verification commands live in [architecture.md](architecture.md) and [testing.md](testing.md).
