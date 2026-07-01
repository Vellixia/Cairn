---
title: Documentation Conventions
type: guide
status: living
updated: 2026-07-01
---

# Documentation Conventions

Read this before creating any new `.md` file in this repo. It tells you where a doc goes,
which template to start from, how to name it, and how it stays consistent over time.

This applies to **humans and AI agents alike**. If you are an AI agent, you were probably
routed here from [AGENTS.md](../AGENTS.md) — that file is your entry point; this file is the
detailed rulebook.

## The organizing principle

`docs/` is organized by **the reader's question**, not by document type. Before creating a
doc, ask "what question is this answering?" — that answer names the folder.

| Folder | Answers the question | Lifecycle |
|--------|----------------------|-----------|
| `docs/reference/` | "How does Cairn work?" | living — edited in place as the system evolves |
| `docs/guides/` | "How do I do X with Cairn?" | living — edited in place |
| `docs/testing/` | "How is Cairn verified?" | mixed — plans/overview living; run logs frozen |
| `docs/planning/` | "Where is Cairn going?" | roadmap living; version plans frozen at release |
| `docs/audits/` | "What did a point-in-time review find?" | frozen snapshot + dated fix-status updates |
| `docs/archive/` | "What used to be true?" | frozen — superseded docs retire here |
| `docs/_templates/` | (not reader-facing) | skeletons for new docs |

`docs/README.md` is the catalog — every doc in the library, one screen. `docs/CONVENTIONS.md`
(this file) is the authoring rulebook. Neither is a technical doc itself.

## Which template do I use?

| I am writing… | type | goes in | start from | filename pattern |
|---|---|---|---|---|
| a version implementation plan | `plan` | `docs/planning/plans/` | `_templates/plan-template.md` | `vMAJOR.MINOR.PATCH.md` |
| an evergreen "how it works" doc | `reference` | `docs/reference/` | `_templates/reference-template.md` | `kebab-case.md` |
| an operator/user how-to | `guide` | `docs/guides/` | `_templates/guide-template.md` | `kebab-case.md` |
| a design decision record | `adr-log` entry | append to `docs/reference/decisions.md` | `_templates/adr-template.md` | `ADR-NNN` (sequential) |
| a point-in-time audit | `audit` | `docs/audits/` | `_templates/audit-template.md` | `kebab-case.md` |
| a live end-to-end walk of one surface | `walk` | `docs/testing/live-e2e/` | `_templates/testing-walk-template.md` | `NN-surface-name.md` |
| a bug or feature gap found while testing | `finding` | `docs/testing/findings/open/` | `_templates/finding-template.md` | `kebab-title.md` |
| a raw test-run log (not a template — just capture what happened) | `run-log` | `docs/testing/findings/runs/<date>-<run-name>/` | — | as produced |

If nothing above fits, ask before inventing a new type — most things do fit one of these.

## Frontmatter

Every doc (except run-logs, which are raw captures) opens with YAML frontmatter:

```yaml
---
title: <Human Title>
type: index | reference | vision | roadmap | guide | plan | adr-log | audit | testing | walk | finding | run-log | template
status: living | draft | released | superseded | open | resolved | fixed | archived
updated: YYYY-MM-DD
version: <e.g. 0.8.0>        # optional — version/snapshot docs only
supersedes: <relative-path>  # optional — the doc this one replaces
severity: low | medium | high | critical   # findings only
---
```

`type` and `status` are **closed vocabularies** — use exactly one of the listed values so the
catalog and any future tooling can rely on them. Don't invent new ones without updating this
file first.

## Naming rules

- **Lowercase kebab-case** for everything: `web-auth.md`, not `WEB.md` or `Web_Auth.md`.
- **Version plans:** `vMAJOR.MINOR.PATCH.md` inside `docs/planning/plans/`.
- **ADRs:** sequential `ADR-NNN`, entries appended to `docs/reference/decisions.md` — never
  their own file.
- **Findings:** short kebab-case description of the bug/gap, e.g. `command-palette-needs-ctrl-k.md`.
- **Run logs:** grouped under a dated folder, `YYYY-MM-DD-<run-name>/`, filenames inside are
  whatever the run naturally produces (e.g. per-route logs).
- **Live E2E walks:** `NN-surface-name.md`, numbered in walk order (see `docs/testing/live-e2e/`
  for the existing sequence — continue it, don't restart numbering).

## Lifecycle rules

- **Living docs** (`reference/`, `guides/`, `roadmap.md`, `testing/overview.md`, etc.) are
  edited in place as the system changes. Update `updated:` when you touch one.
- **Snapshot docs** (version plans, audits) are **frozen** once released — never rewritten
  after the fact. If a plan needs a new direction, write a new file and set `supersedes:` to
  point at the old one; move the old one to `docs/archive/`.
- **Findings** move `open/ → resolved/` when fixed (update `status:` and add the fix
  reference), and the registry at `docs/testing/findings/README.md` is updated in the same
  change. Findings are never deleted — a resolved finding is a regression-test reminder.
- **Archived docs** stay in `docs/archive/` permanently as historical record; they are not
  linked from active navigation (README/AGENTS/catalog) except from a "history" note.

## The managed-block rule

`AGENTS.md` and `CLAUDE.md` (repo root) each contain a block:

```
<!-- BEGIN CAIRN -->
...
<!-- END CAIRN -->
```

This block is **auto-generated by the `cairn rules` CLI** (`crates/cairn-client/src/rules.rs`).
**Never hand-edit content inside these markers** — the next `cairn rules` run will silently
overwrite your edit anyway. Edit everything else in those files freely.

## Root-meta files

Six files stay at the **repository root** (not under `docs/`) because tooling (GitHub, package
registries, this repo's own `cairn rules`) expects them there:

`README.md` · `AGENTS.md` · `CONTRIBUTING.md` · `CLAUDE.md` · `SECURITY.md` · `CHANGELOG.md`

Two of them are **navigators**, not just documents:

- **`README.md`** routes **users** — install, quick start, "where do I go for X".
- **`AGENTS.md`** routes **AI agents** — dev commands, workspace map, "where do I read/write
  for task X", and links here for doc-authoring rules.

When you add a new doc that a user or agent would plausibly look for, add a row to the
relevant routing table in whichever navigator applies (or both), and to the catalog in
[docs/README.md](README.md).

## Quick checklist for a new doc

1. What question does it answer? → pick the folder.
2. Pick the matching template from `docs/_templates/`, copy it, fill in frontmatter.
3. Name it lowercase-kebab (or the pattern above for plans/ADRs/findings/walks).
4. Write the content — don't invent new sections beyond the template unless the template
   truly doesn't fit.
5. Link it from `docs/README.md`, and from `README.md`/`AGENTS.md` if it's something a user
   or agent would look for.
6. If it replaces an older doc, set `supersedes:` and move the old one to `docs/archive/`.
