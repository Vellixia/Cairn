#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "usage: $0 --binary-dir <cargo-test-binary-directory> [--workspace <repository-root>]" >&2
}

die() {
  echo "feature002_network_isolation_error=$1" >&2
  exit 69
}

resolve_test_binary() {
  local binary_dir="$1"
  local test_name="$2"
  local candidate

  if [[ -x "$binary_dir/$test_name" ]]; then
    printf '%s\n' "$binary_dir/$test_name"
    return
  fi

  while IFS= read -r candidate; do
    [[ -x "$candidate" ]] || continue
    case "$candidate" in
      *.d|*.rlib|*.rmeta|*.pdb) continue ;;
    esac
    printf '%s\n' "$candidate"
    return
  done < <(find "$binary_dir" -maxdepth 1 -type f -name "$test_name-*" -print | LC_ALL=C sort)

  die "missing_prebuilt_test_$test_name"
}

run_inside() {
  local bundle="$1"
  local mechanism="${CAIRN_ISOLATION_MECHANISM:-unknown}"
  local proof_dir="$bundle/proofs"

  mkdir -p "$proof_dir"
  printf 'filesystem-ok\n' > "$proof_dir/filesystem.txt"
  grep -Fqx 'filesystem-ok' "$proof_dir/filesystem.txt"
  echo "local_filesystem=available"

  mkdir -p "$proof_dir/local-git"
  git -C "$proof_dir/local-git" init --quiet
  git -C "$proof_dir/local-git" config user.name "Cairn isolation"
  git -C "$proof_dir/local-git" config user.email "isolation@cairn.invalid"
  printf 'local-only\n' > "$proof_dir/local-git/proof.txt"
  git -C "$proof_dir/local-git" add proof.txt
  git -C "$proof_dir/local-git" commit --quiet -m "local isolation proof"
  git -C "$proof_dir/local-git" rev-parse --verify HEAD >/dev/null
  echo "local_git=available"

  if timeout 3 bash -c 'exec 3<>/dev/tcp/1.1.1.1/80' 2>/dev/null; then
    die "external_network_reachable"
  fi
  if timeout 3 getent ahostsv4 example.com >/dev/null 2>&1; then
    die "external_dns_reachable"
  fi
  echo "external_network=unreachable"

  export CAIRN_FEATURE001_FIXTURE_DIR="$bundle/fixtures/databases"
  export RUST_BACKTRACE=0
  "$bundle/bin/feature002_migration_acceptance" --nocapture
  "$bundle/bin/feature002_quickstart" --nocapture
  echo "local_ipc=available"
  "$bundle/bin/feature002_replay" --nocapture
  "$bundle/bin/feature002_privacy" --nocapture

  echo "isolation_mechanism=$mechanism"
  echo "feature002_migration_acceptance=pass"
  echo "feature002_quickstart=pass"
  echo "feature002_mixed_replay=pass"
  echo "feature002_privacy=pass"
  echo "feature002_network_isolated=pass"
}

if [[ "${1:-}" == "--inside" ]]; then
  [[ $# -eq 2 ]] || die "invalid_inside_arguments"
  run_inside "$2"
  exit 0
fi

binary_dir=""
workspace=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary-dir)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      binary_dir="$2"
      shift 2
      ;;
    --workspace)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      workspace="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done

[[ -n "$binary_dir" ]] || { usage; exit 2; }
[[ -d "$binary_dir" ]] || die "binary_directory_not_found"
if [[ -z "$workspace" ]]; then
  workspace="$(git rev-parse --show-toplevel 2>/dev/null)" || die "workspace_not_found"
fi
[[ -f "$workspace/fixtures/databases/feature-001-v1.sqlite3" ]] || die "feature001_fixture_not_found"
[[ -f "$workspace/fixtures/databases/feature-001-v1.manifest.json" ]] || die "feature001_manifest_not_found"

bundle="$(mktemp -d "${TMPDIR:-/tmp}/cairn-feature002-isolated.XXXXXX")"
trap 'rm -rf "$bundle"' EXIT
mkdir -p "$bundle/bin" "$bundle/fixtures/databases"
cp "$0" "$bundle/harness.sh"
chmod +x "$bundle/harness.sh"
cp "$workspace/fixtures/databases/feature-001-v1.sqlite3" "$bundle/fixtures/databases/"
cp "$workspace/fixtures/databases/feature-001-v1.manifest.json" "$bundle/fixtures/databases/"

for test_name in \
  feature002_migration_acceptance \
  feature002_quickstart \
  feature002_replay \
  feature002_privacy
do
  cp "$(resolve_test_binary "$binary_dir" "$test_name")" "$bundle/bin/$test_name"
  chmod +x "$bundle/bin/$test_name"
done

echo "prebuilt_binary_directory=$binary_dir"
echo "isolated_bundle=$bundle"

if [[ "$(uname -s)" == "Linux" ]] && command -v unshare >/dev/null 2>&1; then
  if unshare -n -- true >/dev/null 2>&1; then
    echo "selected_isolation=unshare_network_namespace"
    CAIRN_ISOLATION_MECHANISM="unshare -n" \
      unshare -n -- "$bundle/harness.sh" --inside "$bundle"
    exit 0
  fi
  echo "unshare_network_namespace=unavailable_or_not_permitted"
fi

command -v docker >/dev/null 2>&1 || die "no_network_namespace_or_container_runtime"
docker info >/dev/null 2>&1 || die "container_runtime_unavailable"
container_image="${CAIRN_ISOLATION_CONTAINER_IMAGE:-rust:1-bookworm}"
docker image inspect "$container_image" >/dev/null 2>&1 || die "isolation_container_image_not_preloaded"
echo "selected_isolation=docker_network_none"
docker run --rm --network none \
  --volume "$bundle:/bundle" \
  --workdir /bundle \
  --env CAIRN_ISOLATION_MECHANISM="docker --network none" \
  "$container_image" \
  bash /bundle/harness.sh --inside /bundle
