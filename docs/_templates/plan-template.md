---
title: <Version or Feature Name> Plan
type: plan
status: draft
updated: YYYY-MM-DD
version: <e.g. 0.9.0>
---

# <Title> Plan

## Context

Why this version/feature exists. What problem it solves, what prompted it, the intended
outcome. 2-4 sentences — enough for a reader with zero prior context to understand the "why".

## Goals

- Bullet list of concrete outcomes this plan delivers.

## Scope / Non-goals

**In scope:** ...

**Non-goals:** what this plan deliberately does NOT do (prevents scope creep and keeps
reviewers from expecting things that were never promised).

## Design / Approach

The chosen approach, and briefly why alternatives were rejected. Diagrams welcome if they
clarify data flow or architecture.

## Work Breakdown

Break the work into phases or sprints. Each unit should have:

### <Phase/Sprint N> — <name>

**What:** what is being built
**Why:** why it's needed
**Files:** exact files to create or edit
**Changes:** exact function signatures, struct fields, or code to write
**Done when:** a specific, verifiable condition

Repeat per phase/sprint.

## Verification

Concrete commands or manual steps to confirm the plan's changes work end-to-end.

```bash
# example
cargo build --workspace
cargo test -p <crate>
```

## Definition of Done

- [ ] Checklist of conditions that must all be true before this plan is considered complete.
