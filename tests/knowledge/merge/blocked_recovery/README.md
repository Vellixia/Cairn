Capability refusal against an older server, the server upgrade, and the delivery that follows.

Rule: the refused work is retained as `blocked` — not `failed`, not `delivered` — is retried **zero**
times against a server known to lack the capability, and after the upgrade is delivered exactly once
under its original idempotency key with no user intervention and no manual data repair (FR-418,
SC-331).
