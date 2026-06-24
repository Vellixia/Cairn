# Security Policy

Cairn is a self-hosted service that stores your project memory and (optionally)
shares knowledge between machines. We take its security and privacy posture
seriously.

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Instead, report
privately via GitHub Security Advisories ("Report a vulnerability") on the
repository, or email the maintainers. We aim to acknowledge reports within a few
days.

## Design principles

- **Local-first, private by default.** Nothing leaves your machine or server
  unless you explicitly enable it. There is no telemetry.
- **Secrets are stripped before they travel.** Every sharing / federation /
  pack-publish path runs content through a sanitization step (secret + PII
  redaction) with an explicit consent gate before publishing. The
  `cairn-share::Sanitizer` is the gatekeeper; nothing bypasses it.
- **Lossless by retention.** Compression keeps the full-fidelity original in a
  content-addressed blob store; compressed views are always recoverable rather
  than destructive.
- **Cryptographic authenticity, not obscurity.** Pack signatures use Ed25519
  (ADR-017). Trust is anchored in a small set of trusted author keys, not in a
  closed CA. Revocations cascade via an append-only log that any peer can audit.
- **Conflict-free merge, no silent data loss.** Sync uses vector clocks +
  OR-Set / GCounter CRDTs (ADR-019) instead of last-write-wins. Concurrent
  edits merge deterministically; no edit is silently dropped.
- **Encryption when you want it.** Sync envelopes can be encrypted end-to-end
  with Argon2id-derived ChaCha20-Poly1305 keys (Sprint 15b). The peer sees
  ciphertext + nonce + salt only; the passphrase never leaves the user's
  device.

## Threat model

| Threat | Mitigation | Status |
|---|---|---|
| **Network observer** (compromised log aggregator, MitM who only records) reads sync traffic in transit | TLS via `CAIRN_TLS_CERT` + `CAIRN_TLS_KEY`; E2E encryption via Argon2id + ChaCha20-Poly1305 | Done |
| **Active MitM** swaps a sync envelope for one of their own | TLS certificate pinning; AEAD authentication tag on every E2E-encrypted envelope | Done |
| **Compromised endpoint** â€” attacker has the user's device | Argon2id can't help here; user-level controls (screen lock, device encryption) are the right layer. Out of scope. | Documented |
| **Compromised server** with TLS in place but no E2E | Attacker sees the user's edits in cleartext while inside the box. Mitigation: E2E encryption with a passphrase the server never sees. | Done (opt-in) |
| **Compromised cairn registry** publishes malicious packs | Ed25519 signature verification against `trusted_keys.json`. Unsigned packs are stored but flagged. | Done (Sprint 13) |
| **Malicious peer registries** propagate a poisoned pack | Trust scopes (Sprint 14): a `Local`-only grant cannot publish a `Public` pack. Cross-scope attempts return `ScopeDenied`. | Done (Sprint 14a) |
| **Cascading revocation gap** â€” peer A revokes pack X, peer B keeps using it | Append-only `revocations.jsonl` log + `since=` cursor for fast pull. Subscriber applies events even when no local copy exists (`revoke_if_exists`). | Done (Sprint 14b) |
| **Last-write-wins data loss** when two devices edit offline | Vector clocks + OR-Set / GCounter CRDTs detect concurrent edits and merge them. Concurrent `content` edits are flagged `Concurrent` so the UI can prompt the user. | Done (Sprint 15a) |
| **Forward-secrecy gap** â€” a single long-lived passphrase encrypts everything | Documented limitation. ECDH per-session key exchange is on the v0.6 roadmap (ADR-022). | Planned v0.6 |
| **Per-tenant isolation gap** â€” one admin can read another admin's memories | `OrgId` column on every memory (default `default`); tenant filter applied at query time in `cairn-store` + `cairn-api`. Multi-tenant mode toggled by `CAIRN_MULTI_TENANT=1`. Integration test `tenant_isolation_filters_recall_by_org` proves no cross-tenant recall. | Done (Sprint 19a, commit `d69d3c4`) |

## Current scope & hardening status

This is mature but still-evolving software. Areas still being hardened (tracked
on the [Roadmap](docs/ROADMAP.md)):

- **CAIRN_INSECURE=1 escape hatch** â€” only for trusted local/private networks.
  Bypasses the TLS gate; do not use on a public network.
- **Collective knowledge / federation** â€” peer trust is opt-in via the
  registry's `trusted_keys.json`. Sanitization + consent pipeline is enforced at
  every publish boundary; no peer can bypass it.
- **Multi-tenancy** â€” per-tenant isolation is shipped in v0.5.0. Every memory
  carries an `OrgId` (`crates/cairn-core/src/tenant.rs`); tenant filter is
  applied at query time in `cairn-store` and `cairn-api`. Toggle via
  `CAIRN_MULTI_TENANT=1`. The `OrgId::default()` keeps the single-tenant
  behaviour unchanged when the flag is off.

When in doubt, run Cairn on a trusted network (LAN/VPN such as Tailscale)
behind your own boundary.

## Cryptographic primitives

- **Ed25519** for pack signatures (Sprint 13). Library: `ed25519-dalek` ~2.1.
- **Argon2id** for passphrase â†’ key derivation (Sprint 15b). Parameters: 64 MiB
  memory, 3 iterations, 1 lane (OWASP-recommended minimum for interactive use).
- **ChaCha20-Poly1305** for AEAD envelope encryption (Sprint 15b). 12-byte
  nonce + 32-byte key + 16-byte tag per message. Library: `chacha20poly1305`
  ~0.10.
- **HMAC-SHA256** for the cost-savings ledger (Sprint 5). The same
  `CAIRN_SECRET_KEY` that signs JWT device tokens signs the ledger â€” both are
  bound to the same server identity.

## Hardening checklist

For production deployments:

1. Set `CAIRN_SECRET_KEY` to a â‰¥32-byte random value (`openssl rand -base64 48`).
2. Set `CAIRN_TLS_CERT` and `CAIRN_TLS_KEY` to a valid PEM pair (e.g. `mkcert`).
3. Bind to `127.0.0.1` or `localhost`, or use a reverse proxy. Never bind a
   plain-HTTP Cairn to a public address with `CAIRN_INSECURE=1`.
4. Enable E2E encryption for sync (`cairn sync --e2e --passphrase â€¦`).
5. Audit `~/.config/cairn/.env` and the docker `.env` â€” never commit
   `MINIO_ROOT_PASSWORD` defaults, never use `minioadmin/minioadmin`.
6. Rotate `CAIRN_SECRET_KEY` at least once a year; rotating invalidates all
   device tokens AND the savings ledger HMAC signatures. Plan for a brief
   re-onboarding window.
7. Subscribe to GitHub Security Advisories on the repo.
