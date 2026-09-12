-- Local schema v10 — the spool binds to a server *instance*, not a URL.
--
-- `event_spool` and `command_spool` were account-bound and nothing else. That
-- closes the identity half of FR-790 — one account's queued work is never
-- delivered under another's credential — and leaves the deployment half of
-- FR-791 open: two different servers reachable at one address are two different
-- servers, and queued work belongs to exactly one of them.
--
-- The gap is not hypothetical. A deployment restored from backup onto a fresh
-- database, or a new one stood up at an address that used to serve another,
-- both present as "the same URL" and mint their own `server_instance_id`. Rows
-- queued against the first would have been delivered to the second, which is
-- the blend FR-495 and FR-496 already forbid for team knowledge — enforced
-- there by `bind_team_server_instance_tx` — and which nothing enforced here.
--
-- **An endpoint is not an identity.** That sentence is already written into
-- `AuthenticatedContext::is_this_peer`, and this column is what lets the spool
-- honour it too.
--
-- Nullable, and the null has one specific meaning: this row was queued before
-- this store had ever established a server instance. It is not "any instance"
-- and it is never treated as a wildcard on the claim. The first drain that runs
-- against an established instance binds those rows to it, once, inside the
-- claim transaction — the safe first-binding rule. A row that already carries
-- an instance is never rebound by that path.
--
-- The one rebinding that does exist is provisional → reported: a lane opened
-- against a server below schema 3 is keyed by an id derived from the endpoint,
-- because such a server reports none, and when that peer is upgraded in place
-- it starts reporting a real id (`sync-namespaces.md` §11a). The lane re-keys,
-- and the rows queued under the provisional id re-key with it. That is the same
-- server, newly able to say so — not a second one.
ALTER TABLE event_spool ADD COLUMN server_instance_id TEXT;
ALTER TABLE command_spool ADD COLUMN server_instance_id TEXT;

-- The claim predicate's index, in the order the predicate reads it. Both halves
-- of the identity are in the key, because a claim that matched the account and
-- filtered the instance afterwards would scan another deployment's backlog to
-- discard it.
DROP INDEX IF EXISTS event_spool_claim;
CREATE INDEX event_spool_claim
    ON event_spool (state, account_id, server_instance_id, next_attempt_at);
DROP INDEX IF EXISTS command_spool_claim;
CREATE INDEX command_spool_claim
    ON command_spool (state, account_id, server_instance_id, next_attempt_at);
