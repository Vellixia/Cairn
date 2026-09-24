# V1 deletion baseline

Frozen comparison commit: `37417192f316cb495601edf44d5bddfa967b0e5c`.

Run `bash scripts/v1-baseline.sh` for reproducible source and surface inventory. The script reads the frozen tree without changing it and compares it with the current worktree. Production includes Rust under `crates/*/src` with guarded test items excluded, plus TypeScript/TSX under `web/{app,components,hooks,lib}`. Tests, fixtures, generated output, dependencies, and deleted narrative specifications earn no production-reduction credit.

Measured at `5589a25a0e2aa0eb1a308cc9f2c0cc7ec1773ec5`:

| Production source | Frozen | V1 | Delta |
|---|---:|---:|---:|
| Rust | 82,783 | 46,775 | -36,008 |
| Web | 9,256 | 7,851 | -1,405 |
| Total | 92,039 | 54,626 | -37,413 |

Measured surfaces: three CLI enum entries (`setup` plus hidden `hook` and `mcp`), five MCP tools, nine web pages, and 61 explicit HTTP/typed web-operation declarations. Applied migrations remain immutable history, so raw migration table counts are not runtime edge-schema counts.

Current architecture and verification commands live in [architecture.md](architecture.md) and [testing.md](testing.md).
