---
title: <Audit Name> Audit
type: audit
status: draft
updated: YYYY-MM-DD
---

# <Audit Name> Audit

**Scope:** what was audited (crates, subsystems, surface area)
**Date:** YYYY-MM-DD
**Commit:** <short sha the audit was run against>

## Method

How the audit was performed (manual review, automated scan, tool used, criteria applied).

## Findings

| ID | Severity | Area | Status |
|----|----------|------|--------|
| AUDIT-1 | critical\|high\|medium\|low | ... | open |

## Detail per finding

### AUDIT-1: <short title>

**Symptom:** what was observed.
**Risk:** why it matters / what could go wrong.
**Recommendation:** the suggested fix or mitigation.

Repeat per finding.

## Fix-status update (dated)

Append a dated subsection each time findings are re-checked, rather than editing the
original findings above. This audit is a frozen snapshot — track remediation here.

### YYYY-MM-DD update

- AUDIT-1: fixed in <commit/PR> — or — still open, tracked in <link>.
