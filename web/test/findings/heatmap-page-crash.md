# Fix: `/memory/heatmap` no longer crashes

Same root cause as `architecture-page-crash.md`. See that file for the
diff and verification. Tracked separately because the heatmap flow
checklist (06 architecture-report-and-heatmap) reported the symptom on
both pages, and a future regression in one but not the other should not
hide behind a single finding.

**Status:** Resolved 2026-06-30.