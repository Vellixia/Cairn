# Testing

Use smallest test that crosses changed boundary.

- Pure logic stays beside implementation under `#[cfg(test)]`.
- SQLite behavior uses a real temporary store in-process.
- Edge delivery tests drive typed spools, receipts, retry, saturation, cache age,
  identity changes, and privacy refusal.
- Server integration tests use `CAIRN_TEST_DATABASE_URL` and real PostgreSQL.
- Web tests type-check generated contract, build production bundle, then exercise live
  desktop and mobile routes.

Required local gates:

```bash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cd web
npm run typecheck
npm run check:api-contract
npm run build
```

`scripts/network-isolated-tests.sh` builds with network access, then runs edge suites
inside a container with external networking disabled. PostgreSQL suites are outside
that gate because server access is required by design.

Tests for removed CLI commands, Task behavior, local canonical knowledge, pull sync,
namespace authority, and cutover routes do not belong in V1. Durable behavior remains
covered at surviving store, daemon, server, and browser boundaries.

A test needing PostgreSQL prints a skip when `CAIRN_TEST_DATABASE_URL` is absent. Such
a skip is not evidence of runtime correctness; release CI must provide PostgreSQL.
