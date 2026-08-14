# Recorded vendor lifecycle payloads

One directory per adapter. Each payload is a realistic vendor lifecycle event, recorded from
the vendor's own official documentation or source, with the source and date recorded in that
directory's `SOURCES.md`.

These fixtures are what makes a vendor change visible in a diff rather than as a silent
behavioral regression (plan.md risk table). They run hermetically: no vendor binary, no
authentication, no network (FR-204, SC-124).

Two halves are asserted for every adapter (D40 tier 3):

1. every capability the profile claims **guaranteed** produces its canonical event;
2. every capability the profile does **not** claim produces nothing, from any payload.
