One task at a shared state; machine A changes AC-1 offline, machine B changes AC-2 offline.

Rule: both criterion changes are present on both machines, neither overwrote the other, and both
machines compute an **identical** `task_state_digest` while their `local_revision` counters differ and
are never compared (FR-490, FR-493, SC-330).
