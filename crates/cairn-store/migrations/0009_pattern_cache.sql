-- Feature 005 — the local pattern cache, local schema v9 (FR-703, FR-710,
-- FR-712a; knowledge-commands.md §3.3 "Retrieval and cache").
--
-- Additive only, in the same sense v8 was: one new table, no existing table
-- changed, no existing row rewritten.
--
-- Three things a reader of this table has to know.
--
-- **1. This is a cache, and never an authority.** Every row here is a copy of a
-- `shared_patterns` row the server already accepted, refilled from the patterns
-- lane like any other non-authoritative copy. Deleting this table — deleting
-- the whole local store — loses nothing the server holds: the next pull writes
-- it back (FR-703, FR-704). Nothing may originate here. A pattern the user is
-- promoting is a `pattern_promote` row in `command_spool` until the server
-- accepts it, because a local record with no path to acceptance is an
-- alternative truth and FR-709 forbids one.
--
-- **2. It is a separate table from `reusable_patterns` because it has to be.**
-- The obvious move — pull the server's copy back into the table patterns
-- already live in — cannot be done honestly. `reusable_patterns` declares
-- `signals`, `signal_digest`, `origin_ref` and `sanitization_report` NOT NULL
-- (0005_project_intelligence.sql), and those, with `source_memory_id` and
-- `origin_deleted`, are exactly the six field names the privacy boundary
-- refuses (FR-708b, knowledge-commands.md §3.3). The server is never sent them
-- and therefore never sends them back. Storing a pulled pattern in that table
-- would mean inventing values for six columns to satisfy NOT NULL — fabricating
-- content in order to record content, which is the one thing a cache must not
-- do. So the safe shape gets its own table, and `reusable_patterns` keeps its
-- meaning unchanged: the local, pre-promotion rows, which stay local (FR-707).
--
-- **3. `trust` is CHECKed to one value for the same reason the server's column
-- is.** `sanitized` means "this passed the privacy gate", and passing the gate
-- is the only thing the server can witness. `validated` and `contested` are
-- derived from `pattern_applications`, which never leave the machine (FR-707),
-- so a row that arrived from the server has no evidence for either and a
-- column able to hold one would be a place for an unearned claim to sit
-- (FR-708g). Local trust stays on the local row and is reported as
-- machine-local rather than canonical.

-- ---------------------------------------------------------------------------
-- The cache table
-- ---------------------------------------------------------------------------

-- The owner's server-held patterns, as this machine last saw them.
--
-- The columns mirror `shared_patterns`' safe shape and stop there. There is no
-- `domain` column: `shared_patterns` CHECKs its own to `personal` because a
-- pattern is a personal-domain record and Feature 005 introduces no fourth
-- domain (FR-708c), and a cache repeating a constant the server has already
-- fixed would only give it a second place to disagree.
--
-- `owner_user_id` is NOT NULL and is part of every read and every write. One
-- machine may legitimately hold more than one identity's cache side by side —
-- a user account is per-server, so two servers are already two owners — and
-- owner-only visibility (FR-708d) is not a filter that may be forgotten at the
-- call site.
--
-- `cached_at` is when *this copy* was written, which is not `updated_at`: that
-- one is the server's statement about the record, this one is the local
-- statement about the copy, and telling a reader the cache is stale needs the
-- second (FR-710, FR-710a).
--
-- `forgotten_at` carries the tombstone in the shape personal knowledge already
-- uses: set, with the content columns emptied. A forgotten pattern stops being
-- read rather than disappearing, so a later pull cannot resurrect it by
-- arriving before the forget does.
CREATE TABLE cached_patterns (
  pattern_id    TEXT PRIMARY KEY,
  owner_user_id TEXT NOT NULL,
  title         TEXT NOT NULL,
  problem       TEXT NOT NULL,
  root_cause    TEXT NOT NULL,
  approach      TEXT NOT NULL,
  constraints   TEXT NOT NULL DEFAULT '[]',
  applicability TEXT NOT NULL DEFAULT '[]',
  trust         TEXT NOT NULL DEFAULT 'sanitized' CHECK (trust = 'sanitized'),
  content_key   TEXT NOT NULL,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  forgotten_at  TEXT,
  cached_at     TEXT NOT NULL,
  UNIQUE (owner_user_id, content_key)
);

-- `pattern_id` is UUIDv5(owner_user_id ‖ content_key), so this constraint and
-- the primary key state the same rule from two directions, exactly as they do
-- on the server. Both are kept here too: the pair is what a reader can check
-- without deriving a UUID, and it is what makes a repeated refill converge on
-- one row rather than accumulating copies (FR-708f).

-- Every read is "this owner's patterns that are not forgotten", which is this
-- index in the order the predicate asks for it. It is the same index
-- `shared_patterns_owner` is on the server, for the same query.
CREATE INDEX cached_patterns_owner ON cached_patterns (owner_user_id, forgotten_at);
