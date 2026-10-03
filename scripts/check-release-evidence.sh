#!/usr/bin/env bash
set -euo pipefail

version=${1:?usage: scripts/check-release-evidence.sh VERSION SOURCE_SHA}
source_sha=${2:?source SHA is required}
file="${CAIRN_RELEASE_EVIDENCE_DIR:-.github/release-evidence}/v${version}.md"

test -f "$file" || { echo "missing candidate evidence: $file" >&2; exit 1; }
python3 - "$file" "$source_sha" <<'PY'
import re, sys

report = open(sys.argv[1], encoding="utf-8").read()
expected = sys.argv[2]
sha = re.search(r"^Candidate source SHA: `?([0-9a-f]{40})`?$", report, re.M)
if not sha or sha.group(1) != expected:
    sys.exit("candidate evidence is not bound to this exact source SHA")

required = (
    "Workspace Rust, format, clippy",
    "Strict PostgreSQL suites",
    "Web contract, type, production build",
    "Browser/setup primary journey",
    "Same-origin deployment and denied origin",
    "Alpha.7 WAL upgrade, transfer, and recovery",
    "Safety, deadline, limits, and readiness",
    "Server/web image build and digests",
    "Native archives and installed-artifact smoke",
    "Claude Code/Codex package setup/repair/restart/compaction",
    "Five paired usefulness scenarios",
)
rows = {}
for name, result, evidence in re.findall(r"^\| ([^|]+) \| ([^|]+) \| ([^|]+) \|$", report, re.M):
    name, result, evidence = name.strip(), result.strip(), evidence.strip()
    if name in rows:
        sys.exit(f"duplicate evidence gate: {name}")
    rows[name] = (result, evidence)
for name in required:
    if name not in rows or rows[name][0] != "PASS" or rows[name][1] in ("", "Candidate not pinned."):
        sys.exit(f"required candidate gate is not proven: {name}")
for name, (result, _) in rows.items():
    if result in ("FAIL", "NOT RUN") and name != "macOS Intel installed artifact":
        sys.exit(f"candidate gate is not proven: {name}")
PY
