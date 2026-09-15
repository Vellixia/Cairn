#!/usr/bin/env bash
# Print reproducible, machine-local V1 baseline measurements. Read-only.
set -euo pipefail
root=${BASELINE_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
cd "$root"
frozen_commit=37417192f316cb495601edf44d5bddfa967b0e5c
git cat-file -e "$frozen_commit^{commit}"
inventory_capture=$(mktemp)
frozen_tree=$(mktemp -d)
trap 'rm -f "$inventory_capture"; find "$frozen_tree" -depth -delete' EXIT
# Build output in a regular file, then emit it once complete. Hashing below can
# safely read closed capture. Avoid async process-substitution writers; normal
# stdout content stays unchanged.
exec 3>&1
exec > "$inventory_capture"
lines() {
  local paths=()
  while IFS= read -r path; do paths+=("$path"); done
  if ((${#paths[@]} == 0)); then printf '%s\n' 0; else wc -l "${paths[@]}" | awk 'END { print $1 }'; fi
}
git archive "$frozen_commit" | tar -x -C "$frozen_tree"
frozen_rust_source=$(find "$frozen_tree/crates" -path '*/src/*.rs' -type f -print | lines)
frozen_rust_production=$(find "$frozen_tree/crates" -path '*/src/*.rs' -type f -print0 | xargs -0 python3 scripts/v1-rust-loc.py)
frozen_rust_inline_tests=$((frozen_rust_source - frozen_rust_production))
frozen_web_production=$(find "$frozen_tree/web/app" "$frozen_tree/web/components" "$frozen_tree/web/hooks" "$frozen_tree/web/lib" -type f \( -name '*.ts' -o -name '*.tsx' \) -print | lines)
frozen_rust_tests=$(find "$frozen_tree/crates" "$frozen_tree/tests" -type f -name '*.rs' \( -path '*/tests/*' -o -path "$frozen_tree/tests/*" \) -print | lines)
frozen_web_tests=$(find "$frozen_tree/web/e2e" -type f \( -name '*.ts' -o -name '*.tsx' \) -print | lines)
frozen_fixtures=$(find "$frozen_tree/tests" "$frozen_tree/crates" -type f \( -path '*/fixtures/*' -o -path '*/corpora/*' -o -path '*/budget/*' -o -path '*/merge/*' \) -print | lines)
rust_source=$(find crates -path '*/src/*.rs' -type f -print | lines)
# Rust-aware lexer masks rustdoc/comments/literals before locating complete
# guarded items. It is self-tested below, including braces/attributes in text.
rust_production=$(find crates -path '*/src/*.rs' -type f -print0 | xargs -0 python3 scripts/v1-rust-loc.py)
rust_inline_tests=$((rust_source - rust_production))
web_production=$(find web/app web/components web/hooks web/lib -type f \( -name '*.ts' -o -name '*.tsx' \) -print | lines)
rust_tests=$(find crates tests -type f -name '*.rs' \( -path '*/tests/*' -o -path 'tests/*' \) -print | lines)
web_tests=$(find web/e2e -type f \( -name '*.ts' -o -name '*.tsx' \) -print | lines)
fixtures=$(find tests crates -type f \( -path '*/fixtures/*' -o -path '*/corpora/*' -o -path '*/budget/*' -o -path '*/merge/*' \) -print | lines)
printf '# V1 baseline measurements\n\n'
printf 'frozen baseline commit: %s\n' "$frozen_commit"
printf 'current comparison tree: working tree based on %s\n\n' "$(git rev-parse HEAD)"
printf '| category | frozen | current | delta | method |\n|---|---:|---:|---:|---|\n'
printf '| Rust production (`crates/*/src/*.rs`, guarded test items removed) | %s | %s | %+d | Rust-aware lexer + physical-line count |\n' "$frozen_rust_production" "$rust_production" "$((rust_production - frozen_rust_production))"
printf '| Inline Rust test modules | %s | %s | %+d | excluded from production |\n' "$frozen_rust_inline_tests" "$rust_inline_tests" "$((rust_inline_tests - frozen_rust_inline_tests))"
printf '| Web production (`web/{app,components,hooks,lib}` TS/TSX) | %s | %s | %+d | `find` + `wc -l` |\n' "$frozen_web_production" "$web_production" "$((web_production - frozen_web_production))"
printf '| Rust tests | %s | %s | %+d | excluded from production |\n' "$frozen_rust_tests" "$rust_tests" "$((rust_tests - frozen_rust_tests))"
printf '| Web E2E tests | %s | %s | %+d | excluded from production |\n' "$frozen_web_tests" "$web_tests" "$((web_tests - frozen_web_tests))"
printf '| fixtures/corpora | %s | %s | %+d | excluded from production |\n\n' "$frozen_fixtures" "$fixtures" "$((fixtures - frozen_fixtures))"
cli_top_level=$(awk '/^enum Command \{/{on=1;next} on && /^}/{exit} on && /^    [A-Z][A-Za-z0-9_]+/{n++} END{print n}' crates/cairn/src/main.rs)
mcp_tools=$(awk '/pub const TOOL_NAMES/{on=1} on && /^];/{exit} on && /"cairn_/{n++} END{print n}' crates/cairn/src/mcp.rs)
raw_http_routes=$(rg '^\s*\.route\(' crates/cairn-server/src/api.rs crates/cairn-server/src/events.rs | wc -l | tr -d ' ')
# Most browser routes bind through the typed `.web_operation` registry, not a
# literal `.route` call. Count both source forms so route inventory cannot
# silently omit registry-bound endpoints.
web_operations=$(python3 - <<'PY'
import re
source = open("crates/cairn-server/src/api.rs").read()
print(len(re.findall(r'operation!\(\s*[A-Z][A-Z_]*\s*,\s*"(?:GET|POST|PATCH|DELETE)"', source)))
PY
)
http_routes=$((raw_http_routes + web_operations))
web_pages=$(find web/app -name page.tsx | wc -l | tr -d ' ')
tables_from_sources() {
  python3 scripts/v1-rust-loc.py --table-names "$@" | sort -u
}
pg_tables=$(tables_from_sources crates/cairn-server/migrations crates/cairn-server/src/db.rs)
sqlite_tables=$(tables_from_sources crates/cairn-store/migrations crates/cairn-store/src/migrate.rs)
pg_count=$(printf '%s\n' "$pg_tables" | sed '/^$/d' | wc -l | tr -d ' ')
sqlite_count=$(printf '%s\n' "$sqlite_tables" | sed '/^$/d' | wc -l | tr -d ' ')
printf 'frozen surfaces: CLI top-level=32 MCP=6 HTTP=62 web pages=18 pg tables=40 sqlite tables=53\n'
printf 'current surfaces: CLI top-level=%s MCP=%s HTTP=%s (raw=%s + web_operation registry=%s) web pages=%s pg tables=%s sqlite tables=%s\n\n' "$cli_top_level" "$mcp_tools" "$http_routes" "$raw_http_routes" "$web_operations" "$web_pages" "$pg_count" "$sqlite_count"
printf '%s\n' 'route declarations (source-located):'
rg -n '\.route\(' crates/cairn-server/src/api.rs crates/cairn-server/src/events.rs
printf '%s\n' 'per-item inventory (source | item | disposition):'
awk '
  function braces(s, a, b) { a=s; b=s; return gsub(/{/, "", a)-gsub(/}/, "", b) }
  /^enum [A-Za-z0-9_]+/ { name=$2; sub(/ \{.*/, "", name); depth=1; next }
  name != "" { if (depth == 1 && /^    [A-Z][A-Za-z0-9_]*/) { item=$1; sub(/[,({].*/, "", item); disposition="keep"; if (name == "Command" && item ~ /^(Init|Verify|Link|Unlink|Traits)$/) disposition="replace"; if (name == "Command" && item ~ /^(Pattern|Privacy|Delete)$/) disposition="simplify"; print "CLI|crates/cairn/src/main.rs|" name "::" item "|" disposition }; depth += braces($0); if (depth == 0) name="" }
' crates/cairn/src/main.rs
awk '
  /\.route\(/ { collecting=1; text=$0 }
  collecting && !/\.route\(/ { text=text " " $0 }
  collecting && /\)[,;]?[[:space:]]*$/ {
    gsub(/[[:space:]]+/, " ", text)
    disposition = text ~ /auth\/register|projects\/\{id\}\/join/ ? "remove" : "keep"
    print "HTTP|" FILENAME "|" text "|" disposition
    collecting=0
  }
' crates/cairn-server/src/api.rs crates/cairn-server/src/events.rs
python3 - <<'PY'
import re
source = open("crates/cairn-server/src/api.rs").read()
for name, method, path in re.findall(r'operation!\(\s*([A-Z][A-Z_]*)\s*,\s*"(GET|POST|PATCH|DELETE)"\s*,\s*"([^"]+)"', source):
    # Browser registry operations are live routes. Every one is deliberately
    # retained or simplified in compact UI; no implicit route disposition.
    disposition = "simplify" if name in {"ACTIVITY", "RETRIEVAL_TRACES", "INTEGRATION_HEALTH", "SYNC_STATUS"} else "keep"
    print(f"HTTP|web_operations::{name}|{method} {path}|{disposition}")
PY
python3 - <<'PY'
import re
source = open("crates/cairn/src/mcp.rs").read()
for name, body in re.findall(r'"name": "(cairn_[^"]+)"(.*?)(?=\n        json!\(|\n    \])', source, re.S):
    if name in {"cairn_context", "cairn_search"}:
        print(f"MCP|crates/cairn/src/mcp.rs|{name}|keep")
    action = re.search(r'"action": \{.*?"enum": \[(.*?)\]', body, re.S)
    if action:
        for value in re.findall(r'"([^"]+)"', action.group(1)):
            print(f"MCP|crates/cairn/src/mcp.rs|{name}::{value}|keep")
PY
while IFS= read -r page; do
  case "$page" in *'/activity/'*|*'/agents/'*|*'/domains/'*|*'/retrievals/'*|*'/sync/'*) disposition=replace;; *) disposition=keep;; esac
  printf 'WEB|%s|%s|%s\n' "$page" "$page" "$disposition"
done < <(find web/app -name page.tsx | sort)
while IFS= read -r table; do printf 'POSTGRES|migration+db.rs|%s|keep\n' "$table"; done <<< "$pg_tables"
while IFS= read -r table; do printf 'SQLITE|migration+migrate.rs|%s|keep\n' "$table"; done <<< "$sqlite_tables"
python3 - <<'PY'
import re
from pathlib import Path
daemon = Path("crates/cairnd/src/main.rs").read_text()
server = Path("crates/cairn-server/src/main.rs").read_text()
tick = daemon[daemon.index("tokio::spawn(sync::run_worker"):daemon.index("Ok(daemon)")]
daemon_jobs = sorted(set(re.findall(r"\b((?:sync|recover|verify)::[A-Za-z_][A-Za-z0-9_]*)\s*\(", tick)))
server_jobs = re.findall(r"tokio::spawn\((consolidate::[A-Za-z_:]+)\(", server)
dispositions = {
    "sync::run_worker": "keep", "recover::reap_idle_sessions": "simplify",
    "recover::sweep_pending_handoffs": "keep", "verify::sweep_projects": "simplify",
    "consolidate::Consolidator::new": "keep",
}
for path, symbol in [("crates/cairnd/src/main.rs", x) for x in daemon_jobs] + [("crates/cairn-server/src/main.rs", x) for x in server_jobs]:
    if symbol not in dispositions:
        raise SystemExit(f"unclassified named job source: {symbol}")
    disposition = dispositions[symbol]
    print(f"JOB|{path}|{symbol}|{disposition}")
PY
printf '%s\n' 'web pages:'
find web/app -name page.tsx | sort
printf '%s\n' 'PostgreSQL tables:'
printf '%s\n' "$pg_tables"
printf '%s\n' 'SQLite tables:'
printf '%s\n' "$sqlite_tables"
printf 'crate graph:\n'
cargo metadata --no-deps --format-version 1 | jq -r '.packages[] | select(.name|startswith("cairn")) | "- " + .name + " -> " + ([.dependencies[] | select(.path != null) | .name] | join(", "))'
printf '%s\n' 'artifacts:'
toolchain=/Users/andresholivin/.rustup/toolchains/1.97.1-aarch64-apple-darwin/bin
if [[ -x "$toolchain/cargo" && -x "$toolchain/rustc" ]]; then
  printf 'release command: RUSTC=%s/rustc %s/cargo build --release --bins\n' "$toolchain" "$toolchain"
else
  printf '%s\n' 'release command: cargo +1.97.1 build --release --bins (or set RUSTC to Rust >=1.97)'
fi
for artifact in target/release/cairn target/release/cairnd target/release/cairn-server; do
  if [[ -f "$artifact" ]]; then printf '%s bytes %s\n' "$(wc -c < "$artifact" | tr -d ' ')" "$artifact"; else printf 'unavailable %s (run cargo build --release --bins)\n' "$artifact"; fi
done
if [[ -d web/.next ]]; then du -sk web/.next | awk '{print $1 * 1024 " bytes web/.next"}'; else printf '%s\n' 'unavailable web/.next (run npm --prefix web run build)'; fi
printf '%s\n' 'Rust LOC parser self-check:'
python3 scripts/v1-rust-loc.py --self-check
exec >&3
exec 3>&-
cat "$inventory_capture"
printf '%s\n' 'comparison rule: frozen measurements above remain immutable; current inventory is emitted with an explicit disposition per item.'
