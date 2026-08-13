#!/usr/bin/env bash
# Every requirement in the spec is cited by a test, or is listed here as a
# known exception with a reason.
#
# The point is traceability, not a coverage percentage. A requirement that no
# test names cannot be checked when it changes: the suite passes, the product
# works, and nobody can answer "which test proves this?" without reading
# everything. See `docs/testing.md`.
#
# The exception list is deliberately tiny. Ten requirements were uncited when
# this check was written, and seven of them were already *proved* by an existing
# test that simply never named them — so they were cited rather than excused,
# which cost one doc comment each. What is left below cannot be discharged by a
# test at all. The list is meant to stay this short: an entry is an admission
# that nothing checks the requirement, so it needs a reason beside it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPEC="$ROOT/specs/001-cairn-mvp/spec.md"

# Requirements not cited by any test today.
#
# Grouped by why. Anything added here needs a line saying which group it joins.
KNOWN_UNCITED=(
  # Constraints on what Cairn must NOT do or NOT need. A passing suite is
  # compatible with these being violated, so a test is the wrong instrument:
  # they need a guard over the dependency graph and the UI surface instead.
  # Tracked as guards in `docs/testing.md`, not as coverage.
  FR-025 FR-063

  # Time-to-first-value: a measurement about a person, not a code path.
  SC-001
)

fail=0
missing=()

spec_ids() {
  grep -oE '\b(FR|SC)-[0-9]{3}\b' "$SPEC" | sort -u
}

cited_ids() {
  # Citations count only where a *test* makes them. A requirement named in a
  # doc comment on production code documents an intention; it does not pin the
  # behaviour, which is the whole point of the exercise.
  {
    grep -rhoE '\b(FR|SC)-[0-9]{3}\b' "$ROOT/tests" "$ROOT/web/e2e" 2>/dev/null

    # Inline `#[cfg(test)]` modules are tests too. They sit at the end of the
    # file by convention throughout this codebase, so everything from the first
    # `#[cfg(test)]` onwards is test code — which is why this is a line filter
    # and not a full parse.
    find "$ROOT/crates" -name '*.rs' -print0 2>/dev/null |
      xargs -0 awk '/#\[cfg\(test\)\]/ { intest = 1 } intest { print }' 2>/dev/null |
      grep -oE '\b(FR|SC)-[0-9]{3}\b'
  } | sort -u
}

known() {
  printf '%s\n' "${KNOWN_UNCITED[@]}" | sort -u
}

uncited=$(comm -23 <(spec_ids) <(cited_ids | sort -u))

for id in $uncited; do
  if ! known | grep -qx "$id"; then
    missing+=("$id")
    fail=1
  fi
done

# A stale allowlist is its own problem: an entry for a requirement that is now
# cited quietly grants an exemption nobody needs, and hides the next regression
# behind it.
stale=()
while read -r id; do
  [[ -z "$id" ]] && continue
  # `grep -qxF` against the list on stdin: quoting `$uncited` into a single
  # printf argument made this compare against one blob and never match, which
  # reported every entry as stale.
  if ! grep -qxF "$id" <<<"$uncited"; then
    stale+=("$id")
  fi
done < <(known)

if ((${#missing[@]})); then
  echo "Requirements with no test citing them, and not listed as known:"
  printf '  %s\n' "${missing[@]}"
  echo
  echo "Cite each one in the doc comment of the test that proves it, e.g."
  echo "    /// The daemon reconciles sessions from a previous run (FR-009)."
  echo "If a test is the wrong instrument, add it to KNOWN_UNCITED in"
  echo "$0 with a reason."
fi

if ((${#stale[@]})); then
  echo "Listed as uncited, but now cited by a test — remove from KNOWN_UNCITED:"
  printf '  %s\n' "${stale[@]}"
  fail=1
fi

if ((fail == 0)); then
  total=$(spec_ids | wc -l | tr -d ' ')
  exempt=$(known | wc -l | tr -d ' ')
  echo "Requirement traceability: $((total - exempt)) of $total cited by tests, $exempt known exceptions."
fi

exit "$fail"
