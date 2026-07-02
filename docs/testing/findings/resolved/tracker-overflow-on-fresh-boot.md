---
title: "Finding: `GotchaTracker::record` and `FollowupTracker::record` panic on freshly-booted systems"
type: finding
status: resolved
updated: 2026-07-01
severity: medium
---

# Finding: `GotchaTracker::record` and `FollowupTracker::record` panic on freshly-booted systems

**Discovered:** 2026-06-30, while rewriting `crates/cairn-tests/tests/01_memory_tiers.rs`.

**Severity:** Medium (server crash on every failure event in the first hour after boot; the live cairn server has been up for 44 minutes when this was found).

**Reproduction:**

```rust
let mut t = cairn_memory::GotchaTracker::new(); // default window = 1 hour
t.record(FailureEvent::new("topic", "ctx"));
// panic: overflow when subtracting duration from instant
```

The root cause is at `crates/cairn-memory/src/gotcha_tracker.rs:122` and `crates/cairn-memory/src/followup_tracker.rs:56`:

```rust
let cutoff = Instant::now() - self.window;
```

`Instant - Duration` panics when `self.window` exceeds the system uptime. `GotchaTracker::new()` defaults to a 1-hour window, `FollowupTracker::new()` to 30 seconds. The boot time of the workstation where this was found was 44 minutes.

**Fix applied (this branch):** clamp the cutoff with `checked_sub`. If subtraction would underflow, fall back to the earliest record (or `now` if empty) — neither can be expired, so the loop body is a no-op until records older than `now - window` actually exist.

**Status:** fixed in 0.7.1.

**Tests added:** the rewritten `01_memory_tiers.rs::gotcha_tracker_clusters_repeated_failures` and `followup_tracker_surfaces_repeated_recall_queries` both call `record()` on a freshly-constructed tracker and would have failed before the fix.