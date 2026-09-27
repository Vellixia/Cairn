# Product Requirements Document

> **Product:** Cairn
> **Version:** v0.1.0-alpha.9 target; staged through v0.1.0
> **Status:** Draft
> **Updated:** 2026-09-27

This document defines what Cairn should achieve for users. [Roadmap](roadmap.md) owns version scope, engineering work and release gates. Published alpha.8 is the baseline; this alpha.7 checkout does not establish target acceptance. Targets below are proposals for review, not measured results.

## 1. Overview

**Product promise**

Cairn helps coding agents resume useful work without making developers repeat decisions, failed approaches and project conventions.

**Problem**

A new session, compaction, machine or agent can lose the reasoning needed to continue a project. Developers explain the same constraints again, repeat failed investigations and rediscover procedures. Saving more text alone does not solve this: irrelevant, stale or unsupported guidance can make later work worse.

**Solution**

Cairn retains supported, attributable project knowledge and brings relevant guidance into later agent sessions. Supported capture runs during ordinary work; explicit remembering covers decisions capture cannot observe. Developers can inspect where a claim came from, understand uncertainty and correct it. Shared knowledge stays under authorized human control.

**Intended experience**

Yesterday, a developer established why a workaround failed and which tested procedure worked. Today, another supported agent starts in the same authorized project with relevant guidance and its evidence. The developer continues work instead of repeating the investigation. If the procedure changed or evidence conflicts, Cairn shows that uncertainty rather than presenting an unquestionable answer.

**Goal**

- Improve later work: useful recall and fewer repeated investigations with bounded distraction.
- Make first value reachable through understandable setup, then keep routine memory effort low.
- Make knowledge trustworthy through privacy, provenance, visible uncertainty and correction.

**Non-Goal**

- Full conversation archive, arbitrary reasoning reconstruction or a useful fact from every session.
- Autonomous truth decisions, automatic semantic merging or verified claims without supported evidence.
- Task/project-management system, self-registration, hosted SaaS or enterprise-readiness promises.
- Full offline memory authority: self-hosted shared service is required for fresh recall; outages allow bounded capture and degraded continuity.
- New model/vector/graph infrastructure or a broader human CLI without demonstrated product need.

**Through v0.1.0:** keep one setup path, supported agent integrations and web inspection on the existing canonical-server architecture. Personal/team reuse supports the primary project-continuity job; it is not a separate equal-priority product promise.

---

## 2. Users

### Developer — primary user

- Need: relevant prior decisions, failures and procedures when resuming a repository.
- Problem: session resets and agent switches force repeated explanation or investigation.
- Goal: continue useful work with little routine memory management.

### Project member / knowledge reviewer

- Need: understandable origin, evidence, uncertainty and permission to correct shared knowledge.
- Problem: stale or conflicting advice spreads when nobody can inspect or repair it.
- Goal: make a correction once and see later recall reflect it.

### Administrator / operator

- Need: straightforward deployment, access management and recoverable operation.
- Problem: setup friction, unclear failures or data loss prevent the team trusting Cairn.
- Goal: enable authorized users and maintain the service without guessing.

Initial audience: solo developers and self-hosted teams of 1–10. One person may fill all three responsibilities; reviewer is not a new authorization role. Claude Code and Codex are primary native journeys; other clients receive only capabilities demonstrated in the support matrix.

---

## 3. Core Flow

### First use

1. Operator deploys the supported service and grants account/project access.
2. Developer connects the matching repository using protected credentials and `cairn setup`.
3. Cairn confirms activation or explains the next action needed.
4. Developer works normally; supported capture or explicit remembering produces attributable knowledge.
5. Developer can inspect the first accepted memory. First value is completed when a later task uses relevant guidance, not merely when a record exists.

### Returning to work

1. Developer opens a later session or switches to a supported agent.
2. Cairn supplies bounded, authorized guidance relevant to current project/work.
3. Agent uses useful guidance; developer can inspect why it appeared.
4. Developer corrects outdated knowledge when necessary; later recall respects the recorded correction.

### Failure Flow

1. Setup, capture, delivery or retrieval cannot complete.
2. Cairn distinguishes missing permission, unsupported capability, pending work, outage and normal absence.
3. User sees a safe reason, actual known state and supported next action.
4. User retries or repairs access/configuration; admitted work retains its identity without duplicate effects.
5. If safe recovery is unavailable, coding work continues without fresh memory. Cairn never guesses another session or invents successful delivery.

---

## 4. Requirements

P0 protects trust, privacy, identity and accepted data. P1 completes the user experience. All outcomes remain acceptance obligations; an unchecked outcome does not mean every part is unimplemented.

### PRD-01 — Start without guesswork

**Priority / version:** P1, alpha.9. **Outcome:** operator provisions access and a setup-ready project; developer connects the authorized repository through documented steps. Invalid or ambiguous matching never grants access.

- [ ] **Acceptance:** given fresh deployment and valid access, when developer completes setup and later work, then accepted knowledge is inspectable and relevant guidance is recallable; invalid/nonmember cases explain recovery without changing foreign state.

### PRD-02 — Work with the existing agent

**Priority / version:** P0 configuration safety; P1 activation, alpha.9. **Outcome:** setup preserves user/manager-owned configuration, reports actual supported capabilities and permits safe rerun/removal. Unsupported automation is never advertised as working.

- [ ] **Acceptance:** given existing configuration, when setup is run twice or encounters user edits, then unrelated bytes remain intact, conflicts are visible and supported agent behavior matches activation guidance.

### PRD-03 — Remember useful work safely

**Priority / version:** P0 privacy, alpha.9; P1 usefulness, beta.1. **Outcome:** supported decisions, failures and procedures become attributable knowledge with low routine effort. Explicit remembering covers unobservable decisions and obeys the same privacy/evidence rules. Raw conversations, outputs and credentials are not captured as safe memory.

- [ ] **Acceptance:** given supported useful work, benign work and unsafe input, when capture/consolidation runs, then supported knowledge can be traced, benign work invents nothing and unsafe content is refused without exposure.

### PRD-04 — Recover interrupted work

**Priority / version:** P0, alpha.9. **Outcome:** admitted pending work survives supported interruptions, keeps its original actor/project and reports delivery state. Retry has one canonical effect. Queue limits and possible capture loss are explicit.

- [ ] **Acceptance:** given an interruption after acceptance or a credential change, when work is retried, then no duplicate effect or reattribution occurs and unresolved/lost work is reported honestly.

### PRD-05 — Trust and correct memory

**Priority / version:** P0 access/evidence, alpha.9; P1 complete maintenance, alpha.10. **Outcome:** members see origin, evidence and uncertainty; authorized users correct knowledge while preserving history. Personal ownership, project membership and team governance remain distinct. Reuse elsewhere does not automatically verify a claim here.

- [ ] **Acceptance:** given stale/conflicting knowledge, when an authorized user records correction, then later recall reflects it and history remains inspectable; unauthorized users cannot read private knowledge or mutate protected state.

### PRD-06 — Resume with relevant context

**Priority / version:** P0 identity/deadlines, alpha.9; P1 measured relevance, beta.1. **Outcome:** later sessions receive relevant, bounded guidance or honest absence/outage. Context belongs to the exact caller; cached guidance has visible age. Availability, delivery and understanding are different claims.

- [ ] **Acceptance:** given simultaneous callers, an outage or conflicting/empty knowledge, when a session requests context, then it receives only authorized relevant material with truthful state inside its budget, never a sibling's context or invented confirmation.

### PRD-07 — Understand what happened

**Priority / version:** P1 first-use feedback, alpha.9; complete browsing/operations, alpha.10. **Outcome:** humans find memory/session history, understand evidence and delivery failures, and act without hidden commands or raw-ID copying. Lists disclose bounds; actions explain pending, success and failure.

- [ ] **Acceptance:** given substantial history or a failed action, when user browses or recovers, then records remain reachable under declared paging behavior and next steps are understandable on desktop, mobile and keyboard.

### PRD-08 — Keep knowledge through change

**Priority / version:** P0, alpha.9; declared capacity/recovery envelope, beta.2. **Outcome:** supported upgrade, transfer and restore preserve recoverability, account for source records and never widen access. Unsupported legacy data remains recoverable with explicit disposition. Logical transfer is distinct from physical backup.

- [ ] **Acceptance:** given populated legacy data, concurrent changes or interrupted transfer, when supported upgrade/restore completes, then accepted data and references remain valid, source dispositions are accounted for and credentials/access are not exposed.

### PRD-09 — Rely on advertised support

**Priority / version:** P0, every release. **Outcome:** installed product, guidance and advertised capability agree. Users know which agents/platforms work, current operating limits and supported recovery. Missing verification is a limitation, not a success claim.

- [ ] **Acceptance:** given an advertised installation, when another user follows its guide, then setup, later recall and recovery match the declared version/support scope; required unverified behavior prevents release acceptance.

**Traceability:** these nine outcomes consolidate the prior draft's FR-001–FR-049 and user stories. Historical identifiers retain their original meanings in the [prior technical draft](https://github.com/Vellixia/Cairn/blob/0f1ebb11b120ecf1cc8afff23fec8cbb149cfdd2/docs/product/prd.md); they are not reassigned. PRD-01–PRD-09 remain stable for roadmap references. Implementation acceptance must still cover the detailed obligations relevant to its release.

---

## 5. UI / UX

- **Overview:** understand project activity, accepted knowledge and current continuity/delivery state.
- **Memory:** find relevant claims, inspect origin/evidence, understand selection and record permitted correction.
- **Sessions:** revisit prior work and handoffs; understand missing, incomplete or bounded history.
- **Governance:** review shared proposals, conflicts and lifecycle decisions under actual permissions.
- **Settings / access:** connect projects, manage accounts/membership/tokens and find supported recovery.

Loading, empty, error, success and disabled states must be distinct. Pending is not accepted; offline/cached/stale includes age; refused/unsupported explains the next action. Conflict/needs-recheck remains visible. Preserve recoverable input, prevent accidental repeated submission and confirm irreversible actions. Essential flows work with visible labels/focus, keyboard and mobile layouts.

---

## 6. Data

| Product concept | Information users need | Boundary |
| --- | --- | --- |
| Account / access | Identity, project membership, role, credential state | Access is granted explicitly; secrets never become memory. |
| Project | Repository identity and authorized members | Similar remotes or clone paths do not grant permission. |
| Session / handoff | Which work happened, status and useful continuation | Exact caller attribution; no fabricated prior history. |
| Knowledge / evidence | Claim, scope, origin, support, uncertainty and corrections | Preserve meaning/history; applicability never grants access. |
| Delivery / recovery state | Pending, accepted, refused or failed operation and known age | Retry identity preserved; no fake completion or confirmation. |

Server is canonical shared authority; local delivery/cache is not another knowledge source. Detailed storage fields and wire shapes belong to [technical architecture](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/architecture.md) and implementation contracts, not this product definition.

---

## 7. API

Product exposes one human setup path, supported agent capabilities for context/search/remember/session/handoff, and web inspection/governance/administration. Equivalent actions apply the same identity, privacy and permission rules across interfaces.

Invalid, denied, missing, conflicting and unavailable operations return safe, understandable outcomes. A visible control or a generic tool name does not grant permission or unsupported lifecycle behavior. HTTP routes, JSON examples and transport details remain in [version-pinned server contracts](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/crates/cairn-server/contracts/web-api-v1.ts) and the prior technical draft; release changes require matching implementation documentation.

---

## 8. Security & Reliability

- Only authorized users/agents access the intended project or private knowledge.
- Raw prompts/transcripts, credentials and unsafe material do not enter safe-memory transfer or public diagnostics.
- Explicit remembering cannot forge evidence or bypass privacy; attestation is visibly distinct from system verification.
- Revoked/disabled access is denied; matching cached context is invalidated when denial is observed. Offline silence cannot reveal unseen remote revocation.
- Accepted operations preserve actor/meaning through retries; local admission, server acceptance and actual delivery are separate states.
- Coding work continues through Cairn failure with bounded delay, honest degradation and documented recovery.

---

## 9. Edge Cases

| Situation | Expected user outcome |
| --- | --- |
| New project / no supported useful fact | Clear absence, not invented memory or an outage warning. |
| Invalid input / duplicate request / concurrent correction | Actionable refusal or one recorded effect; conflicting claims stay inspectable. |
| Network/service failure / stale cache | Work continues; pending/unavailable/age shown; fresh search not pretended. |
| Wrong actor/project / revoked access | Deny and preserve privacy; no guessed session or silent reassignment. |
| Edited integration / unsupported agent | Preserve user configuration; explain conflict or supported manual alternative. |
| Saturation / partial upgrade or transfer | Account for retained/refused/lost work; offer tested recovery without broadening access. |

---

## 10. Non-Functional

**Low interruption:** supported capture and context obey configured end-to-end budgets; alpha.9 validates current 250ms/1,500ms defaults. Default context budget is 3,000 tokens. More material is not inherently better.

**Compatibility:** primary native journeys are Claude Code/Codex; OpenCode and generic MCP retain demonstrated capability limits. Web supports desktop/mobile/keyboard; native OS/browser claims require installed proof. Hosting remains self-hosted through 0.1.0.

**Scale and performance:** first target is solo/small-team operation. Beta.2 tests a declared envelope with interactive API p95 ≤500ms and primary usable page p95 ≤2s under specified conditions. Capacity, retention, transfer and backup targets live in [versioned roadmap](roadmap.md#v010-beta2--supported-capacity-retention-and-recovery), keeping this document focused on experience rather than machine sizing.

---

## 11. Testing

Validate first-use and returning-session journeys, relevant correction, safe absence, interrupted recovery, permissions/privacy and supported-agent behavior. Test what the user can achieve, not merely whether a record or API response exists.

Use frozen paired tasks with and without Cairn for usefulness; include benign, stale/conflicting and unsafe cases. [Roadmap release gates](roadmap.md#release-checklist) own engineering checks, installed-artifact proof and candidate evidence. Earlier or infrastructure-skipped tests do not accept a new release.

---

## 12. Success Criteria

**Primary success:** a developer resumes later work with relevant supported guidance, avoids repeated investigation and retains the ability to inspect/correct what Cairn knows.

| Metric | Draft target | Release |
| --- | --- | --- |
| Activation | Three fresh journeys succeed; setup ≤5 minutes once prerequisites exist; clean deployment to inspectable accepted memory ≤20 minutes excluding downloads. Later useful recall assessed separately. | alpha.9 |
| Safe continuity | Five-scenario checkpoint recalls three positive cases, labels stale/conflicting uncertainty and invents nothing in benign case; zero scored privacy/attribution defects. | alpha.9 |
| Useful recall / relevance | ≥80% eligible labelled claims usefully recalled; ≥90% delivered claims relevant and supported. | beta.1 |
| Reduced repeat work | ≥20% median reduction in repeated investigation actions; task-completion rate no lower than control. | beta.1 |
| Harm / trust | Zero scored privacy leaks, wrong-actor attribution or high-impact harmful advice; stale/conflicting cases cannot fabricate certainty/evidence. | Every release; broader beta.1 corpus |

Beta.1 uses 30 paired scenarios across at least three repositories and both primary agents. Freeze labels/rubric before measurement; report counts, uncertainty and delivered/total-input token overhead. Score repeat-work reduction only where control investigation count is positive; report absolute counts otherwise. Full method lives in roadmap. These targets are not population-level safety proof. Missed benefit targets require implementation fixes or a narrower supported use case, followed by fresh evaluation. Privacy leaks, wrong-actor attribution and high-impact harmful behavior block release until fixed or affected capability is disabled and remaining scope revalidated; documentation changes alone cannot close these gates. More stored-record counts do not establish value.

---

## 13. Dependencies & Risks

| Dependency / risk | Product consequence | Response |
| --- | --- | --- |
| Self-hosted service and agent integration | Setup burden or lost continuity blocks adoption. | One supported setup journey, demonstrated capabilities and clear fallback/recovery. |
| Supported capture and topic consistency | Valuable decisions may be missed or fragmented. | Explicit remembering, reusable topic guidance when needed and measured recall improvements. |
| Stale/conflicting/irrelevant knowledge | Advice wastes time or causes harmful decisions. | Evidence, visible uncertainty, human correction and paired harm evaluation. |
| Data/access lifecycle | Users lose knowledge or distrust sharing after failure/change. | Explicit permissions, honest state, accountable recovery and tested supported upgrade. |

---

## 14. Open Questions

- [ ] How much explicit recording/review effort will primary developers accept before Cairn costs more than it saves? Measure in pilot; preserve automatic supported capture as default.
- [ ] Which real repositories/workflows best represent the first audience? Freeze evaluation set before scoring, including work that needs no memory.
- [ ] Do draft benefit/harm/activation targets reflect acceptable user value? Ratify before evaluation; narrow supported use cases if evidence misses them.
- [ ] What retention/deletion expectations do users have for accepted history and backups? Set policy before broader cleanup in beta.2.

Version sequence, primary audience, self-hosted authority and default deployment direction are already proposed in roadmap. Implementation mechanics and owner assignments are not unresolved product-positioning questions.

---

## 15. Release Checklist

- [ ] First-use and returning-session flows meet the claimed release scope.
- [ ] Users can inspect origin/uncertainty and perform correction workflows included in the claimed release scope, with visible outcomes.
- [ ] Privacy, permissions, safe absence, bounded interruption and recovery pass.
- [ ] Activation/usefulness targets pass or narrower supported use case is freshly evaluated; safety failures are fixed or affected capability is disabled and remaining scope revalidated under section 12.
- [ ] Advertised support, guidance and operating limits match the delivered product.
- [ ] Version-specific [roadmap gates](roadmap.md#release-checklist) pass and candidate evidence is attached before release acceptance.
