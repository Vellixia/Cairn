One case per authority value and per strict consumer refusal.

    cairn            a deterministic check this machine ran over Cairn-collected evidence
    attested         a verification resting on agent-attested evidence
    remote_cairn     an imported verification the peer established deterministically
    remote_attested  an imported verification the peer established by attestation

Rule: an agent's attestation is never indistinguishable from a check Cairn performed (FR-370). The
two strict consumers — criterion verification (FR-484) and promotion eligibility (FR-396) — refuse
`attested`, `remote_cairn` and `remote_attested`. Where a memory carries both bases, the deterministic
one establishes the authority.
