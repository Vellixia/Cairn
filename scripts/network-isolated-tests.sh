#!/usr/bin/env bash
# SC-006 / FR-045: prove the local product works with no network at all.
#
# `--network none` leaves the container with loopback only, so any code path
# that needed the network would fail rather than quietly degrade. The server
# suites are excluded deliberately: they require a network by design.
set -euo pipefail

IMAGE="${CAIRN_LINUX_IMAGE:-rust:1.97-bookworm}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Warm the shared volumes first, *with* a network: `--offline` below can only
# succeed against a registry that already holds the crates and a target
# directory that already holds the dependencies built. A developer running this
# repeatedly can skip it with CAIRN_ISOLATED_PREPARE=0, which is what keeps the
# local loop fast while leaving CI correct on a cold runner.
if [[ "${CAIRN_ISOLATED_PREPARE:-1}" == "1" ]]; then
  echo "=== warming the registry and target volumes (network allowed) ==="
  docker run --rm \
    -v "$ROOT":/work -w /work \
    -v cairn-linux-target:/tmp/target-linux \
    -v cairn-linux-registry:/usr/local/cargo/registry \
    -e CARGO_TARGET_DIR=/tmp/target-linux \
    -e RUSTUP_TOOLCHAIN=1.97.1 \
    "$IMAGE" \
    bash -lc '
      export PATH=/usr/local/cargo/bin:$PATH
      git config --global --add safe.directory /work
      cargo fetch --locked
      cargo build --workspace --tests
    '
fi

echo "=== running with no network at all ==="
docker run --rm --network none \
  -v "$ROOT":/work -w /work \
  -v cairn-linux-target:/tmp/target-linux \
  -v cairn-linux-registry:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/tmp/target-linux \
  -e RUSTUP_TOOLCHAIN=1.97.1 \
  "$IMAGE" \
  bash -lc '
    export PATH=/usr/local/cargo/bin:$PATH
    git config --global --add safe.directory /work
    echo "--- interfaces available ---"
    ls /sys/class/net
    # Build the binaries the end-to-end suite drives. Testing one package does
    # not build another package binaries, so without this the suite runs
    # whatever cairn/cairnd were left in the cached target directory —
    # silently testing a stale daemon and reporting it as a pass.
    echo "--- building the binaries under test ---"
    cargo build --offline -p cairn -p cairnd
    echo "--- local suites, no network ---"
    cargo test --offline -p cairn-core -p cairn-git -p cairn-store -p cairn-sys \
      -p cairnd \
      -p cairn-e2e --test foundation --test hostile_environment --test concurrency \
      --test us1_capture_handoff --test us1_sessions \
      --test us2_context --test us3_memory --test us4_tasks --test us5_privacy \
      --test manual_mcp_mode --test storage_contention
  '
