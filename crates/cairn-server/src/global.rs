//! Personal and team knowledge on the server: ingest, and the fifth validator
//! entry point (D447, FR-545, FR-577, FR-581, SC-449, SC-456).
//!
//! The four client-side calls to `validate_global_content` close the bypass a
//! client's own code could open. They do nothing about a client that does not
//! run that code at all — modified, out of date, or simply buggy. Any of those
//! can push a `personal_knowledge` or `team_knowledge` item straight at
//! `POST /api/sync/batch`, and before this feature nothing here re-checked it.
//!
//! **A privacy boundary enforced only where the client chooses to enforce it is
//! a convention, not a boundary.** This module is the one entry point that does
//! not trust the other four.

use crate::auth::{AdminUser, SettledUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use cairn_core::domain::{
    ApplicabilityFact, ApplicabilityKind, RelationBasis, RelationKind, ServerRole, TeamState,
};
use cairn_core::validate::{validate_global_content, ProjectIdentity};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

/// The identity tokens to screen a pushed item against.
///
/// The **union of every project the pushing user is a member of** (FR-577). The
/// server cannot know which project the client was working in when it created
/// the item; it can know every project that user could have been in.
///
/// That is deliberately broader than any one client-side check, and it is what
/// catches the case a client-side check structurally cannot: content naming
/// project X, pushed by a client that was working in project Y at the time. The
/// client only ever holds the identity of the project in front of it.
///
/// **Every membership, including the projects since deleted (FR-577a).** This
/// is the one gatherer: the command routes read it through
/// `commands::all_identities_for` and the synchronization entry point calls it
/// directly, so there is one answer to "which projects can this caller be
/// caught naming" rather than two that disagree.
pub async fn identities_for(pool: &PgPool, user_id: Uuid) -> ApiResult<Vec<ProjectIdentity>> {
    let rows = sqlx::query(
        // Membership is the whole filter, and there is deliberately no
        // `deleted_at` predicate beside it. Deleting a project is a soft
        // delete that leaves its name and its remote exactly where they were,
        // while the personal and team knowledge derived from it outlives it
        // untouched (FR-519) — so the name stays disclosable long after the
        // project stops existing, and a screen that let go of it would begin
        // accepting the one disclosure it exists to refuse. Where the reach of
        // a privacy screen is in question the broader answer is the right one
        // (FR-549, D447).
        "SELECT p.name, p.repository_remote
           FROM project_members m
           JOIN projects p ON p.id = m.project_id
          WHERE m.user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut identities = Vec::new();
    for row in &rows {
        let name: String = row.get("name");
        if !name.trim().is_empty() {
            identities.push(ProjectIdentity(name));
        }
        // A remote contributes its host, organisation and repository tokens
        // separately, because content naming any one of them names the project.
        // `git@github.com:acme/widgets.git` yields `github.com`, `acme` and
        // `widgets` rather than only the whole string, which nothing would ever
        // contain verbatim.
        if let Some(remote) = row.get::<Option<String>, _>("repository_remote") {
            identities.extend(remote_identities(&remote));
        }
    }
    // Blank tokens would make the validator refuse everything as
    // `evaluation_incomplete` — they mean "an identity was expected here and is
    // not usable", which is not what an absent project name means.
    identities.retain(|i| !i.0.trim().is_empty());
    identities.sort_by(|a, b| a.0.cmp(&b.0));
    identities.dedup_by(|a, b| a.0 == b.0);
    Ok(identities)
}

/// The host, organisation and repository of a git remote — **the one parser**,
/// shared by every entry point that screens on project identity (FR-546).
///
/// # What a remote contributes, and what it does not
///
/// Three things name a project: the host it lives on, the organisation (or
/// nested groups) it lives under, and the repository itself. A remote also
/// carries syntax — a scheme, an SSH username, a port, a `.git` suffix — and
/// syntax names nothing. `git@github.com:acme/widgets.git` therefore yields
/// `github.com`, `acme` and `widgets`, and neither `git` nor `https`.
///
/// # Position, not vocabulary
///
/// The screen asks whether candidate text *contains* an identity, so a token
/// that identifies nothing refuses everything containing it. Two ways to get
/// that wrong, and this parser is written against both:
///
/// - **Splitting the host apart.** Splitting on `.` turns `github.com` into
///   `github` and `com`, and `com` then refuses `compare`, `command`,
///   `compile` and `component` for every project on GitHub. The host is one
///   identity, kept whole.
/// - **Filtering by word.** Dropping `com`, `git` or `ssh` wherever they
///   appear cures that by creating a hole: `https://github.com/com/net.git`
///   has an organisation literally named `com` and a repository literally
///   named `net`, and content naming them names the project. Structure is
///   decided by **where** a token sits, never by what it spells — so a path
///   component is always an identity, and a scheme never is.
///
/// # Shapes
///
/// - `scheme://[user[:pass]@]host[:port]/path` — HTTPS, SSH URL, `git://`.
/// - `[user@]host:path` — SCP-style, which is the default `git clone` writes.
///   Recognised by there being no `/` before the `:`; a port is *not* stripped
///   here, because `host:1234/repo.git` is a path beginning `1234`, not a port.
/// - Anything else is treated as a bare path (a local or filesystem remote):
///   it has no host, and every segment is an identity.
///
/// The terminal `.git` is removed from the last segment **once**, so a
/// repository actually named `git` survives as `git` rather than vanishing.
pub(crate) fn remote_identities(remote: &str) -> Vec<ProjectIdentity> {
    let remote = remote.trim();
    let (authority, path) = split_remote(remote);

    let mut out = Vec::new();
    if let Some(authority) = authority {
        // `user@` and `user:password@` are credentials, not identity. Split at
        // the *last* `@` so a password containing one cannot hide the host.
        let host = match authority.rsplit_once('@') {
            Some((_, host)) => host,
            None => authority,
        };
        // A port only where a port is meaningful — see the SCP note above.
        let host = match host.rsplit_once(':') {
            Some((before, port))
                if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) =>
            {
                before
            }
            _ => host,
        };
        if !host.is_empty() {
            out.push(ProjectIdentity(host.to_string()));
        }
    }

    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let last = segments.len().saturating_sub(1);
    for (i, segment) in segments.iter().enumerate() {
        // `strip_suffix`, not `trim_end_matches`: the latter strips repeatedly,
        // so a repository named `git` (`.../git.git`) came out empty.
        let segment = if i == last {
            segment.strip_suffix(".git").unwrap_or(segment)
        } else {
            segment
        };
        if !segment.is_empty() {
            out.push(ProjectIdentity(segment.to_string()));
        }
    }
    out
}

/// Split a remote into its authority (host part, if it has one) and its path.
fn split_remote(remote: &str) -> (Option<&str>, &str) {
    // A scheme is `letter *( letter / digit / "+" / "-" / "." ) "://"`.
    if let Some(after_scheme) = remote.find("://").and_then(|i| {
        let scheme = &remote[..i];
        let valid = !scheme.is_empty()
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
        valid.then(|| &remote[i + 3..])
    }) {
        return match after_scheme.split_once('/') {
            Some((authority, path)) => (Some(authority), path),
            // `scheme://host` with no path at all.
            None => (Some(after_scheme), ""),
        };
    }

    // SCP-style: a `:` with no `/` before it. `/srv/git/repo.git:8080` is a
    // path, not a host, which is why the order of these two tests matters.
    if let Some((authority, path)) = remote.split_once(':') {
        if !authority.contains('/') && !authority.is_empty() {
            return (Some(authority), path);
        }
    }

    // A bare path: no host to name, every segment an identity.
    (None, remote)
}

/// Why an ingested item was refused.
///
/// A **distinct type**, not a message string (FR-581, SC-456). The client has to
/// tell an ingest refusal from a capability refusal without parsing prose,
/// because their remedies are opposite: a capability refusal becomes deliverable
/// after a server upgrade and must be held; an ingest refusal can never succeed
/// for the same content and must not be. Treating the second as the first holds
/// the item forever, because no upgrade makes a project-identifying value stop
/// naming a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestRefusal {
    /// One of the validator's class names. Never the offending content — this
    /// type has nowhere to put it.
    pub class: &'static str,
}

impl IngestRefusal {
    /// The wire form. `422`, distinct from the capability path's `409`, so a
    /// client branches on the response shape rather than on a string.
    pub fn into_api_error(self) -> ApiError {
        ApiError::new(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "content_rejected",
            format!("refused by the content validator: {}", self.class),
        )
    }
}

/// Screen one pushed personal or team item before anything is written
/// (FR-545, FR-577).
///
/// Called with the transaction still open and **before** any insert, so a
/// refused item leaves no record, no partial record, and nothing to roll back
/// (FR-581). Returning early is stronger than rolling back: there is no window
/// in which the row existed.
pub fn screen_global_item(
    payload: &Value,
    identities: &[ProjectIdentity],
) -> Result<(), IngestRefusal> {
    let content = payload
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let topic_key = payload.get("topic_key").and_then(|v| v.as_str());
    let value_key = payload.get("value_key").and_then(|v| v.as_str());
    let applicability = applicability_of(payload);

    validate_global_content(content, topic_key, value_key, &applicability, identities).map_err(
        |rejection| IngestRefusal {
            class: rejection.class,
        },
    )
}

/// The applicability facts a pushed payload carries.
///
/// An unrecognized `kind` is dropped rather than guessed at: a kind outside the
/// closed vocabulary cannot be constructed, and inventing one here to carry the
/// value through would be a second, looser vocabulary. The value still gets
/// screened as content, because that is what `validate_global_content` does with
/// every free-text field it is handed.
fn applicability_of(payload: &Value) -> Vec<ApplicabilityFact> {
    payload
        .get("applicability")
        .and_then(|v| v.as_array())
        .map(|facts| {
            facts
                .iter()
                .filter_map(|f| {
                    let kind: ApplicabilityKind =
                        f.get("kind").and_then(|v| v.as_str())?.parse().ok()?;
                    let value = f.get("value").and_then(|v| v.as_str())?.to_string();
                    Some(ApplicabilityFact { kind, value })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Upsert one personal knowledge row (FR-582).
///
/// `writer_id` and `writer_seq` are carried in both directions, under the same
/// `UNIQUE (writer_id, writer_seq)` constraint the local store enforces — the
/// invariant holds on both sides rather than being asserted on one. A peer needs
/// to see the sequence to notice that record 7 arrived and record 6 never did.
pub async fn upsert_personal(
    tx: &mut Transaction<'_, Postgres>,
    owner_user_id: Uuid,
    entity_id: Uuid,
    payload: &Value,
) -> ApiResult<()> {
    let rows = sqlx::query(
        "INSERT INTO personal_knowledge
             (id, owner_user_id, knowledge_type, content, topic_key, value_key,
              writer_id, writer_seq, created_at, superseded_by_id, forgotten_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), $9, $10)
         ON CONFLICT (id) DO UPDATE SET
             superseded_by_id = EXCLUDED.superseded_by_id,
             forgotten_at     = EXCLUDED.forgotten_at
         WHERE personal_knowledge.owner_user_id = $2
        RETURNING (xmax = 0) AS inserted",
    )
    .bind(entity_id)
    .bind(owner_user_id)
    .bind(text(payload, "knowledge_type"))
    .bind(text(payload, "content"))
    .bind(opt_text(payload, "topic_key"))
    .bind(opt_text(payload, "value_key"))
    .bind(text(payload, "writer_id"))
    .bind(
        payload
            .get("writer_seq")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    )
    .bind(opt_uuid(payload, "superseded_by_id"))
    .bind(opt_time(payload, "forgotten_at"))
    .fetch_optional(&mut **tx)
    .await?;

    // Zero rows means the id exists and belongs to a different account. Personal
    // knowledge is private to one user, so this is the same class of refusal the
    // project-scoped upserts make: an id supplied by a client is not evidence of
    // ownership.
    let Some(row) = rows else {
        return Err(ApiError::forbidden(
            "that personal knowledge id belongs to a different account",
        ));
    };

    // **Applicability is written on creation and never again** (FR-440: a
    // personal entry is immutable after creation, the tombstone excepted).
    //
    // The conflict branch above touches two lifecycle columns and deliberately
    // leaves content alone — but `store_applicability` used to run
    // unconditionally underneath it, deleting and reinserting the facts. So a
    // client could re-push an id it already owned and move where an existing
    // record applies, without forgetting and recreating it: an immutable record
    // whose *scope* was mutable.
    //
    // `xmax = 0` is the Postgres idiom for "this `RETURNING` row came from the
    // insert, not the conflict update". Checking it is what separates "the
    // record was just created, store its facts" from "the record already existed,
    // touch nothing but the lifecycle".
    let inserted: bool = row.try_get("inserted").unwrap_or(false);
    if inserted {
        store_applicability(
            tx,
            PERSONAL_APPLICABILITY_DELETE,
            PERSONAL_APPLICABILITY_INSERT,
            entity_id,
            payload,
        )
        .await?;
    }
    Ok(())
}

/// Upsert one team knowledge row (FR-459, FR-582).
///
/// Team knowledge is server-wide, so unlike personal knowledge there is no
/// owning account to scope the conflict on. What scopes it instead is the
/// lifecycle: `state` advances only by compare-and-swap on the expected state
/// (D409, FR-454), so an ingest cannot promote a `proposed` row to
/// `authoritative` by pushing a payload that says so. Ratification happens
/// through the administration path and nowhere else (FR-455).
pub async fn upsert_team(
    tx: &mut Transaction<'_, Postgres>,
    proposed_by_user_id: Uuid,
    entity_id: Uuid,
    payload: &Value,
) -> ApiResult<()> {
    // **An ingested team item may create a proposal and may change nothing
    // else** (FR-455, FR-456, FR-515).
    //
    // `state` was already absent from the update list, for the reason that
    // matters most: an agent must never be able to decide its own authority.
    // `superseded_by_id` and `retired_at` were not, and they are the same
    // decision wearing different names — both are administrator acts, reachable
    // only through `POST /api/team/{id}/ratify` and `.../retire`, which are
    // gated by the `AdminUser` extractor.
    //
    // Leaving them updatable here was a privilege escalation, and one this
    // feature's own supersession fix made worse rather than better: once a
    // canonical read excludes `superseded_by_id IS NOT NULL`, any authenticated
    // member could push a payload naming an arbitrary successor and remove a
    // piece of authoritative team guidance from every reader on the server —
    // or push a null and resurrect guidance an administrator had replaced.
    // Neither requires a project membership, because team knowledge is not
    // membership-scoped.
    //
    // So the conflict clause does nothing at all. An id already present means
    // the proposal has already arrived, and redelivery is idempotent by
    // construction rather than by choosing a safe subset of columns to rewrite.
    // The lifecycle travels the other way — server to device, through
    // `GET /api/sync/changes/team` — which is the only direction an
    // administrator's decision can legitimately move.
    let inserted = sqlx::query(
        "INSERT INTO team_knowledge
             (id, knowledge_type, content, topic_key, value_key, state,
              proposed_by_user_id, writer_id, writer_seq, created_at)
         VALUES ($1, $2, $3, $4, $5, 'proposed', $6, $7, $8, now())
         ON CONFLICT (id) DO NOTHING
        RETURNING id",
    )
    .bind(entity_id)
    .bind(text(payload, "knowledge_type"))
    .bind(text(payload, "content"))
    .bind(opt_text(payload, "topic_key"))
    .bind(opt_text(payload, "value_key"))
    // **The authenticated caller, never the payload.** `proposed_by_user_id`
    // used to be read straight out of the pushed item, so a member could name
    // another account as the proposer — falsifying the attribution FR-459 keeps,
    // and making the change feed show that account a proposal it never made as
    // one of its own (FR-464 shows a member their *own* pending proposals).
    // `apply_item` already knows who is pushing; that is the only answer worth
    // storing.
    .bind(proposed_by_user_id)
    .bind(text(payload, "writer_id"))
    .bind(
        payload
            .get("writer_seq")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
    )
    .fetch_optional(&mut **tx)
    .await?;

    // **Applicability travels with the proposal and never after it.**
    //
    // `DO NOTHING` already made an existing row immutable through this path, and
    // `store_applicability` ran underneath it anyway — deleting and reinserting
    // the facts of a row it had just declined to touch. Any member holding an id
    // received from team sync could therefore re-push it with a fresh idempotency
    // key and re-scope authoritative guidance: make it universal, or hide it from
    // selected stacks, with no administrator involved. Applicability is part of
    // what an administrator ratified (FR-460), so changing it is a ratification
    // decision, not an ingest one.
    //
    // `RETURNING id` is `None` exactly when the conflict fired, which is the
    // signal: facts are stored only for a row this statement actually created.
    if inserted.is_some() {
        store_applicability(
            tx,
            TEAM_APPLICABILITY_DELETE,
            TEAM_APPLICABILITY_INSERT,
            entity_id,
            payload,
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Applicability on ingest (FR-434, FR-435, FR-436, FR-460)
// ---------------------------------------------------------------------------

const PERSONAL_APPLICABILITY_DELETE: &str =
    "DELETE FROM personal_knowledge_applicability WHERE personal_id = $1";
const PERSONAL_APPLICABILITY_INSERT: &str =
    "INSERT INTO personal_knowledge_applicability (personal_id, kind, value)
     VALUES ($1, $2, $3) ON CONFLICT DO NOTHING";
const TEAM_APPLICABILITY_DELETE: &str =
    "DELETE FROM team_knowledge_applicability WHERE team_id = $1";
const TEAM_APPLICABILITY_INSERT: &str =
    "INSERT INTO team_knowledge_applicability (team_id, kind, value)
     VALUES ($1, $2, $3) ON CONFLICT DO NOTHING";

/// What a pushed payload says about applicability.
///
/// The two cases are **not** interchangeable, and conflating them silently
/// widens a record's audience. An entry with no facts applies to every project
/// (D411, FR-435), so writing "no facts" where the sender said nothing turns a
/// record that was scoped to `language=rust` into server-wide guidance — the
/// same class of failure as dropping the facts on the floor, arrived at by
/// obeying a payload that never asked for it.
enum Asserted {
    /// No `applicability` key at all. A lifecycle-only push (a tombstone, a
    /// supersession pointer) carries no opinion about the facts, and whatever
    /// the row already has is left alone.
    Nothing,
    /// An `applicability` array, which **is** an opinion — including when it is
    /// empty, which is how a sender says "this applies everywhere".
    Facts(Vec<ApplicabilityFact>),
}

/// The applicability facts a payload asserts, refusing a kind outside the
/// closed vocabulary rather than trying to store it.
///
/// Deliberately stricter than [`applicability_of`], which screens the same
/// field and drops what it cannot parse. The two differ because their jobs do:
/// the screen inspects as much of a malformed payload as it can reach, while a
/// write has to satisfy `personal_knowledge_applicability`'s `kind` CHECK — and
/// a kind the vocabulary does not contain would fail that CHECK as a database
/// error, which reaches the client as `500` and gets retried forever. Named
/// here instead, it is refused once and stays refused.
///
/// `Err` carries the offending kind and nothing else: a rejected kind is a
/// vocabulary name, never content, so it is safe to say out loud — unlike the
/// value beside it, which is exactly what [`IngestRefusal`] declines to quote.
fn asserted_applicability(payload: &Value) -> Result<Asserted, String> {
    let Some(facts) = payload.get("applicability") else {
        return Ok(Asserted::Nothing);
    };
    // A non-array `applicability` is an assertion this function cannot read at
    // all. Treated as "said nothing" rather than "said nothing applies", for
    // the reason `Asserted` documents.
    let Some(facts) = facts.as_array() else {
        return Ok(Asserted::Nothing);
    };
    let mut out = Vec::with_capacity(facts.len());
    for fact in facts {
        let kind = fact.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let kind: ApplicabilityKind = kind.parse().map_err(|_| kind.to_string())?;
        let value = fact
            .get("value")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        out.push(ApplicabilityFact { kind, value });
    }
    Ok(Asserted::Facts(out))
}

/// Write one row's applicability facts, in the transaction that wrote the row.
///
/// **Replaced, not accumulated.** Every push is a redelivery candidate — the
/// idempotency claim in `sync::apply_item` stops the same key applying twice,
/// but a re-created key or a corrected payload for the same id reaches here
/// again, and inserting on top would leave a row carrying the union of every
/// version of itself ever pushed. Applicability is a set, so replacing it with
/// itself is the ordinary case and costs nothing.
///
/// `ON CONFLICT DO NOTHING` covers the one collision the `DELETE` cannot: a
/// payload naming the same `(kind, value)` twice. That is a set with a
/// duplicate written into it, not a conflict between two writers, and refusing
/// the whole item over it would be refusing a payload that means exactly what a
/// deduplicated one means.
///
/// Returning `Err` here leaves the caller's transaction to roll back, which
/// takes the row and the idempotency claim with it — so a corrected payload
/// gets a fresh attempt rather than being reported a duplicate of a write that
/// never happened.
async fn store_applicability(
    tx: &mut Transaction<'_, Postgres>,
    delete_sql: &'static str,
    insert_sql: &'static str,
    entity_id: Uuid,
    payload: &Value,
) -> ApiResult<()> {
    let facts = match asserted_applicability(payload) {
        Ok(Asserted::Nothing) => return Ok(()),
        Ok(Asserted::Facts(facts)) => facts,
        Err(kind) => {
            return Err(ApiError::invalid(format!(
                "applicability kind `{kind}` is outside the `language`/`tool` vocabulary"
            )))
        }
    };
    sqlx::query(delete_sql)
        .bind(entity_id)
        .execute(&mut **tx)
        .await?;
    for fact in &facts {
        sqlx::query(insert_sql)
            .bind(entity_id)
            .bind(fact.kind.as_str())
            .bind(&fact.value)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Read-back (T101, T129, FR-489, FR-582)
// ---------------------------------------------------------------------------

/// **Two routes, not one route with a namespace parameter.**
///
/// The two reads share a cursor convention and nothing else. Personal
/// knowledge is scoped to the calling identity and takes no parameter that
/// could name a subject at all; team knowledge is server-wide, role-filtered,
/// and carries a different set of columns. A single route would have to branch
/// on a string before it could decide either question, which makes the
/// authorization rule a runtime property of a query parameter rather than of
/// the route — and turns a typo'd namespace into a refusal the router would
/// otherwise have answered as `404` for free.
///
/// The decisive argument is the privacy one. `personal` and `team` as *paths*
/// cannot be extended into `?owner=<someone else>`; a namespace parameter is
/// one field away from being an owner selector, which is precisely the shape
/// this feature's security prerequisite spent its time removing (FR-587).
///
/// `since`/`cursor` follow `sync::sync_changes` exactly: an RFC 3339 cursor in,
/// a page bounded by `sync::PAGE`, the newest delivered row's timestamp back
/// out. There is no client-supplied limit, for the same reason that route has
/// none — the page size is the server's business, and a client that could ask
/// for a larger one would be asking for a slower answer.
#[derive(Deserialize)]
pub struct GlobalChangesQuery {
    #[serde(default)]
    pub since: Option<String>,
    /// How many rows to return, clamped to `sync::PAGE`.
    ///
    /// A client that wants smaller pages may ask for them; it may not ask for
    /// larger ones, because the page size is the server's protection and not the
    /// caller's preference. Absent means the full page, so no existing caller
    /// changes behaviour.
    #[serde(default)]
    pub limit: Option<i64>,
}

impl GlobalChangesQuery {
    pub(crate) fn page(&self) -> i64 {
        self.limit
            .unwrap_or(crate::sync::PAGE)
            .clamp(1, crate::sync::PAGE)
    }
}

/// The capability names a client already polls for on `GET /api/version`
/// (`crate::version::SCHEMA_3_CAPABILITIES`).
///
/// The refusal below quotes these rather than describing the missing tables in
/// prose, so the name a client is told it lacks is the same name it would look
/// for in the advertisement.
const PERSONAL_CAPABILITY: &str = "personal_knowledge";
const TEAM_CAPABILITY: &str = "team_knowledge";

/// The schema at which `team_knowledge.revision` exists (migration 5).
///
/// Not a [`require_capability`] floor. A deployment held at schema 3 or 4 has
/// a complete, working `team_knowledge` table and must keep serving it exactly
/// as it did (FR-415) — it simply has no revision to order by, and says so by
/// sending none. Every place that reads or writes the column consults this
/// first; nothing refuses because of it.
pub(crate) const TEAM_REVISION_SCHEMA: i64 = 5;

/// `revision`, or a typed `NULL` under the same name on a deployment held below
/// [`TEAM_REVISION_SCHEMA`].
///
/// Interpolated into a `SELECT` list or a `RETURNING` list so one statement
/// serves both schemas and the decode below is unconditional. `NULL::bigint` and
/// not `0`: zero is a revision, and a client that received it would order
/// against it. `NULL` is the wire's word for "this server has no revision to
/// give", which is what the mirror's fallback path is written for.
fn revision_or_null(schema_version: i64) -> &'static str {
    if schema_version >= TEAM_REVISION_SCHEMA {
        "revision"
    } else {
        "NULL::bigint AS revision"
    }
}

/// Refuse, by capability name, on a deployment whose migration 3 has not run.
///
/// Held-back deployments are a supported configuration, and this codebase has
/// been bitten repeatedly by statements that assume otherwise: a schema-3
/// column selected unconditionally does not fail the new route, it fails the
/// request that happens to touch the same statement. The refusal is checked
/// before any statement is built, so the failure mode is a named `409` rather
/// than an `undefined_table` surfacing as `500`.
fn require_capability(schema_version: i64, capability: &str) -> Result<(), ApiError> {
    if schema_version >= 3 {
        return Ok(());
    }
    Err(ApiError::new(
        StatusCode::CONFLICT,
        "schema_too_old",
        format!(
            "this server is at schema {schema_version} and has no `{capability}`; \
             upgrade it to schema 3"
        ),
    ))
}

/// One page of changes, and how far the cursor may advance after it.
///
/// Generic in the cursor because the two team feeds do not share one. The
/// timestamp-keyed feeds resume from a `(changed_at, id)` [`PageCursor`]; the
/// team pull feed resumes from a [`TeamCursor`], which is a revision. Both are
/// opaque strings to a client, and the type is what stops one being handed to
/// the decoder for the other.
pub struct ChangePage<C = PageCursor> {
    pub items: Vec<Value>,
    pub cursor: C,
}

/// `GET /api/sync/changes/personal` — the caller's own personal knowledge
/// (T101, FR-489).
///
/// **There is no parameter naming an owner, and there cannot be one.** The
/// owner is `user.id()`, taken from the authenticated identity, so the route
/// has no argument through which one account could ask for another's private
/// notes. That is not a check this handler performs — it is the absence of the
/// thing a check would have to guard, which is the only form of this guarantee
/// that a later edit cannot quietly weaken.
pub async fn sync_personal_changes(
    State(state): State<AppState>,
    user: SettledUser,
    Query(q): Query<GlobalChangesQuery>,
) -> ApiResult<Json<Value>> {
    require_capability(state.schema_version, PERSONAL_CAPABILITY)?;
    let since = PageCursor::decode(q.since.as_deref());
    let page = personal_changes(&state.pool, user.id(), since, q.page()).await?;
    Ok(Json(json!({
        "personal": page.items,
        "cursor": page.cursor.encode(),
    })))
}

/// `GET /api/sync/changes/team` — server-wide team knowledge, filtered by what
/// the caller's role permits (T129, FR-463, FR-464).
/// A stable, opaque-to-the-client statement of whose view a team page reflects.
///
/// Two values are compared for equality and nothing else, so the format only
/// has to distinguish every pair of views that produce different feeds — which
/// is exactly (actor, role), the two inputs to `team_changes`'s filter.
fn visibility_fingerprint(user_id: Uuid, role: ServerRole) -> String {
    format!("{user_id}:{}", role.as_str())
}

pub async fn sync_team_changes(
    State(state): State<AppState>,
    user: SettledUser,
    Query(q): Query<GlobalChangesQuery>,
) -> ApiResult<Json<Value>> {
    require_capability(state.schema_version, TEAM_CAPABILITY)?;
    // **Two feeds, two cursor vocabularies, and the schema decides which.** A
    // deployment at `TEAM_REVISION_SCHEMA` or above pages on the monotonic
    // revision, which is the only key a lifecycle write cannot fail to move;
    // one held below it has no such column and keeps the `changed_at` feed it
    // had (FR-415). The cursor is opaque to the client either way, and
    // `TeamCursor::decode` reads a cursor from the other vocabulary as "start
    // again" — so crossing the migration re-delivers this caller's team corpus
    // once and then resumes normally.
    let (items, cursor) = if state.schema_version >= TEAM_REVISION_SCHEMA {
        let page = team_changes(
            &state.pool,
            user.id(),
            user.role() == ServerRole::Admin,
            TeamCursor::decode(q.since.as_deref()),
            q.page(),
        )
        .await?;
        (page.items, page.cursor.encode())
    } else {
        let page = team_changes_by_changed_at(
            &state.pool,
            user.id(),
            user.role() == ServerRole::Admin,
            PageCursor::decode(q.since.as_deref()),
            q.page(),
        )
        .await?;
        (page.items, page.cursor.encode())
    };
    Ok(Json(json!({
        "team": items,
        "cursor": cursor,
        // Which caller's view of the team feed this page was computed for
        // (FR-592, `contracts/sync-namespaces.md` §1a).
        //
        // The filter above is not the same filter for every caller: a `proposed`
        // row reaches its author and any admin, and no one else. So a cursor
        // handed back here is a position in *this* caller's feed, and it stops
        // being a valid position the moment that view widens — a member promoted
        // to admin, or a machine relinked to a second account, would otherwise
        // keep a cursor that has already walked past rows it can now see, and a
        // monotonic cursor never revisits them.
        //
        // The client cannot compute this itself. It can read its own account id
        // from a token, but the role is the server's to state, and role is half
        // of what decides the filter. It comes from `SettledUser` — the
        // authenticated actor — never from anything the caller sent.
        "visibility": visibility_fingerprint(user.id(), user.role()),
    })))
}

// ---------------------------------------------------------------------------
// The control plane's two domain feeds (T110, FR-888, FR-892, FR-893)
// ---------------------------------------------------------------------------

/// How many rows one control-plane page carries, and the most it will carry.
///
/// `web-control-plane.md` §7: twenty-five by default, a hundred at the most,
/// clamped rather than refused. Separate from `sync::PAGE`, which is the pull
/// feeds' bound: a machine draining a namespace wants the largest page the
/// server will give it, and a person reading a panel wants the first screenful.
pub(crate) const VIEW_PAGE_DEFAULT: i64 = 25;
pub(crate) const VIEW_PAGE_MAX: i64 = 100;

/// The bound and the cursor a control-plane list takes.
///
/// `cursor` rather than `since` because the direction is the opposite one. The
/// pull feeds resume forward from a position they have already passed; these
/// pages walk backward from the newest row, and calling both of them `since`
/// would invite a client to hand one route the other's saved position and get a
/// silently empty answer.
#[derive(Deserialize)]
pub struct DomainViewQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

impl DomainViewQuery {
    fn page(&self) -> i64 {
        self.limit
            .unwrap_or(VIEW_PAGE_DEFAULT)
            .clamp(1, VIEW_PAGE_MAX)
    }
}

/// Which `team_knowledge` rows a caller may see, as one SQL fragment.
///
/// **One statement of the rule, used by both readers.** `team_changes` above
/// pages this table forward for a machine and the view below pages it backward
/// for a person; they differ in ordering and in nothing else, and the thing
/// they must not differ in is this. Written as a function taking its own
/// placeholder numbers rather than as a constant, because the two queries bind
/// their parameters in different positions and a constant would have had to be
/// string-patched at each call site — which is the same duplication with an
/// extra step.
///
/// The rule itself is `sync-namespaces.md` §1a and FR-464: a proposal is not
/// yet guidance, so it reaches its author and any administrator and nobody
/// else, while everything that has been through ratification — including a
/// retirement — reaches every authenticated account.
///
/// What would falsify it: a caller who is neither the author nor an
/// administrator seeing a `proposed` row through either reader.
fn team_visibility_predicate(is_admin: &str, actor: &str) -> String {
    format!("({is_admin} OR state <> 'proposed' OR proposed_by_user_id = {actor})")
}

/// `GET /api/personal/knowledge` — the Domains screen's personal panel
/// (FR-888).
///
/// **The owner is the credential, and there is no parameter that could be
/// anything else.** This is the read half of the guarantee
/// [`sync_personal_changes`] already makes on the pull path, and it is made the
/// same way: not by checking an owner argument but by having none. A route with
/// an owner argument and a check is one edit away from a route with an owner
/// argument; a route with no argument is not.
///
/// **Tombstones are excluded here and included there**, and the asymmetry is
/// the point. A cache learns that a record was forgotten only from the row
/// itself, so the pull feed carries it one last time with no content. A person
/// reading a panel is not a cache: a forgotten record has nothing left to show,
/// and listing it would be an empty row whose only content is that something
/// used to be there.
pub async fn personal_knowledge_view(
    State(state): State<AppState>,
    user: SettledUser,
    Query(q): Query<DomainViewQuery>,
) -> ApiResult<Json<Value>> {
    require_capability(state.schema_version, PERSONAL_CAPABILITY)?;
    let limit = q.page();
    let (at, id) = PageCursor::descending_bound(PageCursor::decode_opt(q.cursor.as_deref()));

    let rows = sqlx::query(
        "WITH visible AS (
             SELECT id, knowledge_type, content, topic_key, value_key,
                    writer_id, writer_seq, created_at, superseded_by_id, forgotten_at,
                    created_at AS changed_at
               FROM personal_knowledge
              WHERE owner_user_id = $1 AND forgotten_at IS NULL
         )
         SELECT * FROM visible
          WHERE ($2::timestamptz IS NULL OR (changed_at, id) < ($2, $3::uuid))
          ORDER BY changed_at DESC, id DESC LIMIT $4",
    )
    .bind(user.id())
    .bind(at)
    .bind(id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
    let mut facts = applicability_by_id(&state.pool, PERSONAL_APPLICABILITY_READ, &ids).await?;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            let id: Uuid = row.get("id");
            personal_row_json(
                id,
                row.get("knowledge_type"),
                row.get("content"),
                row.try_get::<Option<String>, _>("topic_key")
                    .ok()
                    .flatten()
                    .as_deref(),
                row.try_get::<Option<String>, _>("value_key")
                    .ok()
                    .flatten()
                    .as_deref(),
                row.get("writer_id"),
                row.get("writer_seq"),
                row.get("created_at"),
                row.try_get("superseded_by_id").ok().flatten(),
                row.try_get("forgotten_at").ok().flatten(),
                &facts.remove(&id).unwrap_or_default(),
            )
        })
        .collect();

    Ok(Json(json!({
        "items": items,
        "cursor": crate::api::view_cursor(&rows, limit, "changed_at", "id"),
        "limit": limit,
    })))
}

/// `GET /api/team/knowledge` — the Domains screen's team panel and the team
/// curation screen's worklist (FR-888, FR-889).
///
/// **A read path only.** `web-control-plane.md` §8 is explicit that no new
/// mutation endpoint is introduced for team curation: ratify and retire already
/// exist as single compare-and-swap statements, and a web-specific handler that
/// read the state, checked it and then updated it would reopen the
/// double-ratification race those statements close and would make "un-retire"
/// expressible (FR-889a). So this route adds a list in front of actions that
/// already exist, and nothing else.
///
/// Visibility is [`team_visibility_predicate`], the same rule the pull feed
/// applies, so a proposal cannot be visible in one reader and not the other.
pub async fn team_knowledge_view(
    State(state): State<AppState>,
    user: SettledUser,
    Query(q): Query<DomainViewQuery>,
) -> ApiResult<Json<Value>> {
    require_capability(state.schema_version, TEAM_CAPABILITY)?;
    let limit = q.page();
    let (at, id) = PageCursor::descending_bound(PageCursor::decode_opt(q.cursor.as_deref()));

    // **This listing keeps ordering on `changed_at`, and the pull feed does
    // not.** The two readers page the same table in opposite directions for
    // different audiences, and what a mis-ordering costs each of them is not
    // the same thing.
    //
    // The pull feed's cursor is durable and monotonic: a change that fails to
    // move the key is a change that device *never* receives, and nothing later
    // contradicts it. That is why it moved to `revision`. This list's cursor
    // lives inside one person's browsing session and is rebuilt from the newest
    // row every time the screen is opened, so the worst a `changed_at`
    // inversion does here is show two rows in the wrong order on one screen —
    // visible, self-correcting on reload, and not a divergence.
    //
    // Against that, "most recently changed first" is the order a curator asked
    // for, and `changed_at` is the only column that means it. `revision` is a
    // write counter: it would sort a row that was merely re-saved above a row
    // that was genuinely retired, and it would change this list's order for
    // every reader the moment the schema moved. So it travels on each row here
    // — a client that wants to order by it can — and it does not order the
    // page.
    let rows = sqlx::query(&format!(
        "WITH visible AS (
             SELECT {TEAM_WIRE_COLUMNS}, {}
               FROM team_knowledge
              WHERE {}
         )
         SELECT * FROM visible
          WHERE ($3::timestamptz IS NULL OR (changed_at, id) < ($3, $4::uuid))
          ORDER BY changed_at DESC, id DESC LIMIT $5",
        revision_or_null(state.schema_version),
        team_visibility_predicate("$1", "$2")
    ))
    .bind(user.role() == ServerRole::Admin)
    .bind(user.id())
    .bind(at)
    .bind(id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(json!({
        "items": team_items(&state.pool, &rows).await?,
        "cursor": crate::api::view_cursor(&rows, limit, "changed_at", "id"),
        "limit": limit,
        // Which caller's view this page reflects, for the reason
        // `sync_team_changes` states: the filter above is not the same filter
        // for every caller, so a position in it belongs to one caller's feed
        // and stops being a position the moment that caller's view widens.
        "visibility": visibility_fingerprint(user.id(), user.role()),
    })))
}

/// A resume position: a timestamp **and** the last id at it.
///
/// **The id is what makes a page boundary safe.** With a timestamp alone, a group
/// of rows sharing one `changed_at` larger than the page limit was split
/// arbitrarily: the page returned some of them, the cursor advanced to that
/// timestamp, and the next request's strict `changed_at > $since` skipped every
/// remaining row at it. Batched tombstones share a `forgotten_at`, and a
/// migration or a bulk ratification shares an instant, so this is reachable
/// rather than theoretical — and the rows it drops are dropped permanently,
/// because nothing ever asks for that instant again.
///
/// Ordering and comparison both use the pair, so a boundary inside a tie group
/// resumes exactly where it stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCursor {
    pub at: chrono::DateTime<chrono::Utc>,
    pub id: Uuid,
}

impl PageCursor {
    /// The beginning of time, before any row.
    fn start() -> Self {
        Self {
            at: chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap(),
            id: Uuid::nil(),
        }
    }

    /// `<rfc3339>|<uuid>`. Opaque to the client, which stores and echoes it.
    pub(crate) fn encode(&self) -> String {
        format!("{}|{}", self.at.to_rfc3339(), self.id)
    }

    /// Parse either form.
    ///
    /// A bare timestamp is a cursor written by a build that had no id half, and
    /// it resumes as `(that instant, nil)` — which re-delivers the rows at that
    /// exact instant once. Every importer is idempotent by id, so a repeat is
    /// free and a skip would not be; that asymmetry is why this is lenient here
    /// and strict about ordering everywhere else.
    pub(crate) fn decode(raw: Option<&str>) -> Self {
        let Some(raw) = raw else {
            return Self::start();
        };
        let (ts, id) = match raw.split_once('|') {
            Some((ts, id)) => (ts, Uuid::parse_str(id).unwrap_or_else(|_| Uuid::nil())),
            None => (raw, Uuid::nil()),
        };
        match chrono::DateTime::parse_from_rfc3339(ts) {
            Ok(at) => Self {
                at: at.with_timezone(&chrono::Utc),
                id,
            },
            // Unparsable reads as the start, matching `sync::sync_changes`
            // verbatim: refusing would strand a client whose stored cursor was
            // written by a version that formatted it differently.
            Err(_) => Self::start(),
        }
    }

    /// The same encoding, parsed as **absent** rather than as the beginning of
    /// time (T108, T110).
    ///
    /// [`Self::decode`] exists for the pull feeds, which walk *forward* from a
    /// position: for them the beginning of time is the right answer to "no
    /// cursor" and also the right answer to "a cursor I cannot read", because
    /// both mean "start again and re-deliver". The control plane's lists walk
    /// **backward** from the newest row, and for them those two answers are
    /// opposites — the beginning of time is the far end of the feed, so
    /// resolving an unreadable cursor to it would hand the reader an empty page
    /// and a dashboard that looks like nothing ever happened.
    ///
    /// So this returns `None` for both absent and unreadable, and a descending
    /// query applies no lower bound at all when it gets `None`. Strict about
    /// the *shape*, for the same reason: a timestamp with no id half cannot
    /// break a tie, and a descending page that re-delivered a tie group would
    /// repeat rows in a list a person is reading rather than in an importer
    /// that is idempotent by id.
    pub(crate) fn decode_opt(raw: Option<&str>) -> Option<Self> {
        let (ts, id) = raw?.split_once('|')?;
        Some(Self {
            at: chrono::DateTime::parse_from_rfc3339(ts)
                .ok()?
                .with_timezone(&chrono::Utc),
            id: Uuid::parse_str(id).ok()?,
        })
    }

    /// The two halves a descending keyset binds, or two `NULL`s when there is
    /// no cursor.
    ///
    /// Returned as a pair so a query can say `($n::timestamptz IS NULL OR
    /// (at, id) < ($n, $n+1))` and have one code path for the first page and
    /// every page after it. A sentinel "end of time" value would work too and
    /// is worse: it puts a magic timestamp into the query plan, and it is wrong
    /// the day a row is written with a clock further ahead than the sentinel.
    pub(crate) fn descending_bound(
        cursor: Option<Self>,
    ) -> (Option<chrono::DateTime<chrono::Utc>>, Option<Uuid>) {
        match cursor {
            Some(c) => (Some(c.at), Some(c.id)),
            None => (None, None),
        }
    }
}

/// A resume position in the **team** pull feed: a revision, and nothing else.
///
/// **One column, no tie-break, because a revision cannot tie.** `PageCursor`
/// needs its id half because a group of rows can share one `changed_at` and a
/// page boundary inside that group is otherwise unresumable.
/// `team_knowledge.revision` is assigned from a sequence and carries a `UNIQUE`
/// index, so a strict `revision > $since` can never step over a row.
///
/// **A cursor this cannot read resumes at the beginning of the feed, and that
/// is the upgrade path.** A cursor written before migration 5 is a
/// `<rfc3339>|<uuid>` position in a differently-ordered feed; there is no
/// revision it corresponds to, and no arithmetic that could invent one. So it
/// reads as revision 0 and the next pull re-delivers the whole team corpus
/// once. That is [`PageCursor::decode`]'s reasoning applied to a wider
/// discontinuity: every importer of this feed is idempotent by id
/// (`merge_synced_team` admits an equal revision precisely so a repeat costs
/// nothing), a repeat is free, and a skip is a silent permanent divergence.
/// The team corpus is server-wide policy, so "the whole corpus" is small.
///
/// The encoding is prefixed rather than a bare integer so the two cursor
/// vocabularies cannot be confused for one another in either direction: a bare
/// `"1234"` would parse as a revision *and* be a plausible truncation of
/// something else, while `rev:1234` is unambiguous and a `PageCursor` string
/// is unambiguously not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamCursor {
    pub revision: i64,
}

impl TeamCursor {
    /// Before every row. A sequence with `MINVALUE 1` never issues 0.
    pub(crate) fn start() -> Self {
        Self { revision: 0 }
    }

    pub(crate) fn encode(&self) -> String {
        format!("rev:{}", self.revision)
    }

    /// Parse `rev:<n>`; anything else — absent, a pre-migration-5
    /// `PageCursor`, a truncated or garbled string — is [`Self::start`].
    ///
    /// Lenient for the reason `PageCursor::decode` is lenient and then some:
    /// refusing would strand a client whose stored cursor was written by a
    /// version that formatted it differently, and there is no answer other than
    /// "start again" that is not a guess about how far this client had already
    /// read.
    pub(crate) fn decode(raw: Option<&str>) -> Self {
        raw.and_then(|r| r.strip_prefix("rev:"))
            .and_then(|n| n.parse::<i64>().ok())
            .map(|revision| Self { revision })
            .unwrap_or_else(Self::start)
    }
}

/// One page of a user's personal knowledge changed after `since`.
///
/// **Ordered by a derived `changed_at`, not by `created_at`.** This table has
/// no `updated_at` and exactly one in-place mutation — the
/// `superseded_by_id`/`forgotten_at` pair, the same cached-pointer exception
/// `memories.superseded_by_id` already has. Ordering on `created_at` alone
/// makes that mutation unreachable: a second device whose cursor has already
/// passed a row's creation would never learn the row was forgotten, so the
/// tombstone this feature carries on the wire could never actually arrive
/// anywhere. `GREATEST` over the row's own timestamps moves a forgotten row
/// back into the page it belongs in. PostgreSQL's `GREATEST` ignores NULL
/// arguments, so a row that was never forgotten sorts on `created_at` exactly
/// as before.
///
/// A bare `superseded_by_id` change still cannot move the cursor, because the
/// column it would have to move on does not exist — supersession is recorded as
/// a pointer with no timestamp of its own. Stated rather than worked around: the
/// fix is a column, not a cleverer query.
///
/// `writer_id` and `writer_seq` travel on every row (§9, FR-582) — the
/// read-back half of what [`upsert_personal`] above already writes on ingest.
/// A peer needs both to notice a gap in this writer's own stream (§9).
pub async fn personal_changes(
    pool: &PgPool,
    owner_user_id: Uuid,
    since: PageCursor,
    limit: i64,
) -> ApiResult<ChangePage> {
    let rows = sqlx::query(
        "WITH changed AS (
             SELECT id, knowledge_type, content, topic_key, value_key,
                    writer_id, writer_seq, created_at, superseded_by_id, forgotten_at,
                    GREATEST(created_at, forgotten_at) AS changed_at
               FROM personal_knowledge
              WHERE owner_user_id = $1
         )
         SELECT * FROM changed
          WHERE (changed_at, id) > ($2, $3)
          ORDER BY changed_at ASC, id ASC LIMIT $4",
    )
    .bind(owner_user_id)
    .bind(since.at)
    .bind(since.id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
    let mut facts = applicability_by_id(pool, PERSONAL_APPLICABILITY_READ, &ids).await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: Uuid = row.get("id");
        items.push(personal_row_json(
            id,
            row.get("knowledge_type"),
            row.get("content"),
            row.try_get::<Option<String>, _>("topic_key")
                .ok()
                .flatten()
                .as_deref(),
            row.try_get::<Option<String>, _>("value_key")
                .ok()
                .flatten()
                .as_deref(),
            row.get("writer_id"),
            row.get("writer_seq"),
            row.get("created_at"),
            row.try_get("superseded_by_id").ok().flatten(),
            row.try_get("forgotten_at").ok().flatten(),
            &facts.remove(&id).unwrap_or_default(),
        ));
    }
    Ok(ChangePage {
        cursor: page_cursor(&rows, since),
        items,
    })
}

/// The `team_knowledge` columns that reach the wire, enumerated.
///
/// **`origin_digest` is not here and there is no column here to select.** It is
/// local-only and must never reach the wire (D434, FR-551); the columns are
/// enumerated rather than taken as `*` so that adding one to the table cannot
/// put it on the wire by default.
///
/// `changed_at` is still computed and still travels, even now that it no longer
/// orders the pull feed: it is what the human listing sorts by, it is honest
/// provenance ("when did this last change"), and a mirror upgraded before its
/// server has nothing else to compare two pages with. `revision` is appended by
/// the caller through [`revision_or_null`], because whether that column exists
/// depends on the schema this deployment actually applied.
const TEAM_WIRE_COLUMNS: &str = "id, knowledge_type, content, topic_key, value_key, state,
                    proposed_by_user_id, ratified_by_user_id, ratified_at,
                    writer_id, writer_seq, created_at, superseded_by_id,
                    retired_by_user_id, retired_at,
                    GREATEST(created_at, ratified_at, retired_at, superseded_at)
                        AS changed_at";

/// One page of team knowledge changed after `since`, as this caller may see it
/// (T129, FR-463, FR-464).
///
/// **Not membership-scoped, and takes no project argument at all** (FR-463). An
/// authoritative team entry is a server-wide default, so an account with zero
/// project memberships reads the same corpus as an account in every project.
/// This is the one read in the feature where `project_members` plays no part,
/// and it is deliberate.
///
/// **State visibility is the role filter** (FR-464): a member sees every entry
/// that has been through ratification plus their own proposals; an admin sees
/// every state, because deciding what to ratify requires seeing what is
/// waiting.
///
/// `retired` is transmitted to members too, which FR-464's shorter phrasing
/// ("authoritative entries and their own proposals") does not spell out. A
/// retirement is not new guidance being shown to a member — it is the
/// withdrawal of guidance that member was already given, and a device holding
/// an authoritative copy that never learns of the retirement keeps serving
/// withdrawn policy forever. Filtering it out would make the retire path a
/// local-only act (FR-456, FR-457).
///
/// **Ordered and paged on `revision`, which is the whole point of this feed
/// having one** (FR-456, FR-457, FR-465). Ratification, retirement and
/// supersession are *the* lifecycle events of this table and all three happen
/// long after `created_at`, so the key has to be something those writes move —
/// which is why this feed was keyed on
/// `GREATEST(created_at, ratified_at, retired_at, superseded_at)` and not on
/// `created_at`.
///
/// That key was not enough, and the gap was not a tie. Those columns are
/// stamped with `now()`, which is **transaction start** time, so a retirement
/// whose transaction opened before an earlier-committing ratification writes a
/// `retired_at` *older* than that `ratified_at` — and `GREATEST` then reads the
/// same value after the retirement as before it. A cursor already at that value
/// never sees the row again, so **no other device ever learns of the
/// retirement**: guidance withdrawn server-wide keeps being served, and nothing
/// later contradicts it because the feed is keyed by the value that failed to
/// move. Measured against this repository's own PostgreSQL as a one-second
/// inversion, with no lock contention required (migration 5 carries the trace).
///
/// `revision` comes from a sequence, is assigned at statement time by a trigger
/// on every row write, and is therefore monotonic in the order the writes
/// actually happened. No write can leave it where it was, so no change can fall
/// behind a cursor.
///
/// Ordered on `revision` **alone**: it is `UNIQUE`, so unlike a timestamp it
/// cannot tie, and a strict `>` cannot step over a row at a page boundary.
///
/// Callers must have established that this deployment is at
/// [`TEAM_REVISION_SCHEMA`]; below it, [`team_changes_by_changed_at`] is the
/// feed.
pub async fn team_changes(
    pool: &PgPool,
    caller_user_id: Uuid,
    caller_is_admin: bool,
    since: TeamCursor,
    limit: i64,
) -> ApiResult<ChangePage<TeamCursor>> {
    let rows = sqlx::query(&format!(
        "SELECT {TEAM_WIRE_COLUMNS}, revision
           FROM team_knowledge
          WHERE revision > $1
            AND {}
          ORDER BY revision ASC LIMIT $4",
        team_visibility_predicate("$2", "$3")
    ))
    .bind(since.revision)
    .bind(caller_is_admin)
    .bind(caller_user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    let cursor = rows
        .last()
        .and_then(|r| r.try_get::<i64, _>("revision").ok())
        .map(|revision| TeamCursor { revision })
        // An empty page leaves the cursor where the caller had it rather than
        // moving it forward, for `page_cursor`'s reason: anything written
        // between the query and the response must still be reachable.
        .unwrap_or(since);
    Ok(ChangePage {
        items: team_items(pool, &rows).await?,
        cursor,
    })
}

/// [`team_changes`] as it was before migration 5, for a deployment held below
/// it (FR-415).
///
/// **Kept, rather than refusing.** A server at schema 3 or 4 has a complete
/// `team_knowledge` table and an operator may deliberately be holding it there
/// while a current binary runs (see [`crate::db::connect`]); making the team
/// feed 409 at that point would take a working corpus away as the price of a
/// fix to how it is ordered. So such a deployment keeps the feed it had —
/// including the defect above, which cannot be fixed without the column it does
/// not have — and says on the wire that it has no revision to offer, which is
/// what the mirror's fallback comparison is written for.
///
/// The predicate and the column list are the same ones [`team_changes`] uses,
/// not copies of them: the thing these two readers must never differ in is
/// *which rows a caller may see*.
pub async fn team_changes_by_changed_at(
    pool: &PgPool,
    caller_user_id: Uuid,
    caller_is_admin: bool,
    since: PageCursor,
    limit: i64,
) -> ApiResult<ChangePage> {
    let rows = sqlx::query(&format!(
        "WITH changed AS (
             SELECT {TEAM_WIRE_COLUMNS}, NULL::bigint AS revision
               FROM team_knowledge
         )
         SELECT * FROM changed
          WHERE (changed_at, id) > ($1, $2)
            AND {}
          ORDER BY changed_at ASC, id ASC LIMIT $5",
        team_visibility_predicate("$3", "$4")
    ))
    .bind(since.at)
    .bind(since.id)
    .bind(caller_is_admin)
    .bind(caller_user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(ChangePage {
        cursor: page_cursor(&rows, since),
        items: team_items(pool, &rows).await?,
    })
}

/// One page of `team_knowledge` rows, as JSON, with their applicability facts.
///
/// Shared by both feeds and by the control-plane listing so that "what a team
/// row looks like on the wire" is decided in exactly one place. Every row must
/// carry the columns in [`TEAM_WIRE_COLUMNS`] plus a `revision` (possibly
/// `NULL`).
async fn team_items(pool: &PgPool, rows: &[sqlx::postgres::PgRow]) -> ApiResult<Vec<Value>> {
    let ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
    let mut facts = applicability_by_id(pool, TEAM_APPLICABILITY_READ, &ids).await?;
    Ok(rows
        .iter()
        .map(|row| {
            let id: Uuid = row.get("id");
            team_wire_row(row, id, facts.remove(&id).unwrap_or_default()).to_json()
        })
        .collect())
}

/// Decode one queried row into the wire shape.
fn team_wire_row(row: &sqlx::postgres::PgRow, id: Uuid, applicability: Vec<Value>) -> TeamWireRow {
    TeamWireRow {
        id,
        knowledge_type: row.get("knowledge_type"),
        content: row.get("content"),
        topic_key: row.try_get::<Option<String>, _>("topic_key").ok().flatten(),
        value_key: row.try_get::<Option<String>, _>("value_key").ok().flatten(),
        state: row.get("state"),
        proposed_by_user_id: row.get("proposed_by_user_id"),
        ratified_by_user_id: row.try_get("ratified_by_user_id").ok().flatten(),
        ratified_at: row.try_get("ratified_at").ok().flatten(),
        writer_id: row.get("writer_id"),
        writer_seq: row.get("writer_seq"),
        created_at: row.get("created_at"),
        superseded_by_id: row.try_get("superseded_by_id").ok().flatten(),
        retired_by_user_id: row.try_get("retired_by_user_id").ok().flatten(),
        retired_at: row.try_get("retired_at").ok().flatten(),
        // The alias every one of these queries selects.
        // `GREATEST` ignores nulls and `created_at` is `NOT NULL`, so the
        // column is always present; `created_at` is the fallback only so a
        // decode failure cannot drop the whole page.
        changed_at: row
            .try_get("changed_at")
            .unwrap_or_else(|_| row.get("created_at")),
        // `NULL` on a deployment below `TEAM_REVISION_SCHEMA`, which
        // `revision_or_null` selected under this name precisely so this decode
        // does not have to know which schema it is reading.
        revision: row.try_get("revision").ok().flatten(),
        applicability,
    }
}

/// How far the cursor may advance after one page.
///
/// The page is ordered ascending on the same column `since` compares against,
/// so the last row delivered is the newest one delivered, and everything not
/// delivered sorts at or after it. An empty page leaves the cursor where the
/// caller had it rather than moving it to `now()`, which would step over
/// anything written between the query and the response.
///
/// This is `sync::page_cursor` with its multi-table arm removed: that function
/// pins the cursor to the smallest last-row timestamp among *four* tables
/// because one truncated page can hide rows older than another table's newest.
/// A single-table page has nothing to be pinned against. It inherits the same
/// tie exposure — `PAGE` rows sharing one timestamp with a `>` cursor would
/// step over the rest — which is unchanged from the route this one follows.
pub(crate) fn page_cursor(rows: &[sqlx::postgres::PgRow], since: PageCursor) -> PageCursor {
    rows.last()
        .and_then(|r| {
            let at = r
                .try_get::<chrono::DateTime<chrono::Utc>, _>("changed_at")
                .ok()?;
            let id = r.try_get::<Uuid, _>("id").ok()?;
            Some(PageCursor { at, id })
        })
        .unwrap_or(since)
}

const PERSONAL_APPLICABILITY_READ: &str =
    "SELECT personal_id AS id, kind, value FROM personal_knowledge_applicability
      WHERE personal_id = ANY($1) ORDER BY id, kind, value";
const TEAM_APPLICABILITY_READ: &str =
    "SELECT team_id AS id, kind, value FROM team_knowledge_applicability
      WHERE team_id = ANY($1) ORDER BY id, kind, value";

/// Every row's applicability facts, in one statement.
///
/// One query for the page rather than one per row: a full page is 500 rows, and
/// 500 round trips to answer one pull is a cost paid on every device's polling
/// interval (FR-489) for a table that holds a handful of rows per record.
async fn applicability_by_id(
    pool: &PgPool,
    sql: &'static str,
    ids: &[Uuid],
) -> ApiResult<std::collections::HashMap<Uuid, Vec<Value>>> {
    let mut out: std::collections::HashMap<Uuid, Vec<Value>> = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let rows = sqlx::query(sql).bind(ids).fetch_all(pool).await?;
    for row in &rows {
        out.entry(row.get("id")).or_default().push(json!({
            "kind": row.get::<String, _>("kind"),
            "value": row.get::<String, _>("value"),
        }));
    }
    Ok(out)
}

/// One `personal_knowledge` row as the wire carries it.
///
/// Scalar arguments rather than a `PgRow`, so this stays a pure function this
/// module's tests can call directly — matching the rest of this file, where
/// every test is pure (no pool, no live database; see `mod tests` below).
#[allow(clippy::too_many_arguments)]
fn personal_row_json(
    id: Uuid,
    knowledge_type: &str,
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    writer_id: &str,
    writer_seq: i64,
    created_at: chrono::DateTime<chrono::Utc>,
    superseded_by_id: Option<Uuid>,
    forgotten_at: Option<chrono::DateTime<chrono::Utc>>,
    applicability: &[Value],
) -> Value {
    json!({
        "id": id,
        "knowledge_type": knowledge_type,
        "content": content,
        "topic_key": topic_key,
        "value_key": value_key,
        "writer_id": writer_id,
        "writer_seq": writer_seq,
        "created_at": created_at.to_rfc3339(),
        "superseded_by_id": superseded_by_id,
        "forgotten_at": forgotten_at.map(|t| t.to_rfc3339()),
        "applicability": applicability,
    })
}

/// One `team_knowledge` row on its way to the wire.
///
/// **Field for field, this is `cairn_store::global::SyncedTeamKnowledge`** —
/// the local mirror `merge_synced_team` takes. Every name here is a field name
/// there, and the two lists are the same length, so a pulled row deserializes
/// into the mirror without a translation layer that could drop something on the
/// way.
///
/// Two fields are deliberately not in both lists, and the asymmetries run in
/// opposite directions. `origin_digest` is on the mirror and not here: it is
/// local to the machine that computed it and never transmitted, so the mirror
/// always stores `NULL` (D434, FR-551). `superseded_at` is on the server table
/// and not here: it exists only so a supersession can move this route's pull
/// cursor, and a device has nothing to do with it — what a device needs is
/// `superseded_by_id`, which does travel.
///
/// One field is in both lists under two names. `changed_at` here is
/// `server_changed_at` on the mirror: the same value, renamed on arrival
/// because on a device the load-bearing fact about it is *whose clock it is*.
/// It is not a column of `team_knowledge` on either side — here it is the
/// derived ordering key both queries compute, there it is what the last
/// applied page's key was recorded as.
///
/// This list is the thing to check when a column is added to either side. It
/// silently lost `retired_by_user_id` once, which made "who retired this" a
/// question only the server could answer (FR-457).
///
/// A named struct rather than [`personal_row_json`]'s positional arguments: at
/// fifteen fields, two of the same type adjacent (`ratified_at`/`retired_at`,
/// `proposed_by_user_id`/`ratified_by_user_id`) would be swappable at the call
/// site with nothing to notice it. Kept pure for the same reason
/// `personal_row_json` is — every test in this module runs without a pool.
struct TeamWireRow {
    id: Uuid,
    knowledge_type: String,
    content: String,
    topic_key: Option<String>,
    value_key: Option<String>,
    state: String,
    proposed_by_user_id: Uuid,
    ratified_by_user_id: Option<Uuid>,
    ratified_at: Option<chrono::DateTime<chrono::Utc>>,
    /// A `TEXT` column holding the writing store's UUID (`FR-490`), handed back
    /// verbatim so the mirror's `Uuid` field parses it. Re-serializing it here
    /// would not make a value some other client invented any more valid, and
    /// would cost the whole page over one row.
    writer_id: String,
    writer_seq: i64,
    created_at: chrono::DateTime<chrono::Utc>,
    superseded_by_id: Option<Uuid>,
    /// Who retired it (FR-457). Emitted for the same reason
    /// `ratified_by_user_id` is: a transition recorded with a timestamp and no
    /// actor is half a record, and the half that is missing is the one an
    /// operator asks for.
    retired_by_user_id: Option<Uuid>,
    retired_at: Option<chrono::DateTime<chrono::Utc>>,
    /// **The version this server ordered the row by** — the same
    /// `GREATEST(created_at, ratified_at, retired_at, superseded_at)` both
    /// queries above already compute to page and sort on, carried rather than
    /// recomputed so the value a device compares against is byte-for-byte the
    /// value this page was sorted by.
    ///
    /// It exists because a device could not previously order a pulled page
    /// against its own concurrent local write. `merge_synced_team` overwrote
    /// unconditionally, so a page fetched *before* a local transition and
    /// applied after it rolled the row back — writing `NULL` over
    /// `retired_by_user_id` and `retired_at` and leaving FR-457's "who acted"
    /// unanswerable on the very machine that acted. The mirror had nothing to
    /// compare because the only clock that can decide the question, the
    /// server's, was never sent. This is that clock.
    ///
    /// One clock, not two: the mirror stores this value and compares the next
    /// page's against *it*, never against a local timestamp.
    changed_at: chrono::DateTime<chrono::Utc>,
    /// **The monotonic server revision, and the field that actually orders**
    /// (FR-456, FR-457, FR-465). `changed_at` above is kept because it is
    /// honest provenance and because a mirror that has only ever spoken to an
    /// older server has nothing else — but it is not a row version, and the
    /// paragraph above overstated what it can do.
    ///
    /// Those lifecycle columns are stamped with `now()`, which is *transaction
    /// start* time. Two writes whose transactions open in one order and commit
    /// in the other therefore record their timestamps in the wrong order, so a
    /// retirement can leave `GREATEST(created_at, ratified_at, retired_at,
    /// superseded_at)` exactly where the preceding ratification left it. Two
    /// distinct states then share one "version": nothing can order them, and
    /// the pull feed — keyed on the same value — never re-sends the row, so no
    /// other device learns of the retirement at all.
    ///
    /// `revision` comes from `team_knowledge_revision_seq`, assigned at
    /// statement time by a trigger on every row write. It is monotonic in the
    /// order the writes happened, whatever their transactions' start times
    /// were, and no write can leave it unchanged. Gaps mean nothing; only the
    /// order does.
    ///
    /// `None` only on a deployment held below [`TEAM_REVISION_SCHEMA`], which
    /// has no such column. It is **not** revision zero: the mirror reads
    /// `None` as "this server cannot order pages for me" and falls back to
    /// comparing `changed_at`, which is what it did before this field existed.
    revision: Option<i64>,
    applicability: Vec<Value>,
}

impl TeamWireRow {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "knowledge_type": self.knowledge_type,
            "content": self.content,
            "topic_key": self.topic_key,
            "value_key": self.value_key,
            "applicability": self.applicability,
            "state": self.state,
            "proposed_by_user_id": self.proposed_by_user_id,
            "ratified_by_user_id": self.ratified_by_user_id,
            "ratified_at": self.ratified_at.map(|t| t.to_rfc3339()),
            "writer_id": self.writer_id,
            "writer_seq": self.writer_seq,
            "created_at": self.created_at.to_rfc3339(),
            "superseded_by_id": self.superseded_by_id,
            "retired_by_user_id": self.retired_by_user_id,
            "retired_at": self.retired_at.map(|t| t.to_rfc3339()),
            "changed_at": self.changed_at.to_rfc3339(),
            // Alongside `changed_at`, never instead of it. A client below
            // local schema 12 reads only `changed_at` and must keep working;
            // one at or above it prefers `revision` and falls back to
            // `changed_at` when this is `null` (`merge_synced_team`).
            "revision": self.revision,
        })
    }
}

// ---------------------------------------------------------------------------
// The team lifecycle: the administration path (FR-453 – FR-457, FR-465)
// ---------------------------------------------------------------------------

/// What a ratification may record alongside itself.
///
/// Absent by default, and an absent body is a valid request: ratifying without
/// superseding anything is the ordinary case, and requiring `{}` of a CLI that
/// has nothing to say would be a syntax tax with no meaning behind it.
#[derive(Deserialize, Default)]
pub struct RatifyBody {
    /// The authoritative entry this ratification replaces (D431, FR-462).
    ///
    /// **Explicit or absent — never inferred.** `classify_proposal` may decide
    /// `duplicates` and `conflicts_with` on its own; it may not decide that one
    /// piece of server-wide policy retires another. That decision has a person
    /// in the loop at exactly the moment it is made, and this field is how the
    /// person says so.
    #[serde(default)]
    pub supersedes: Option<Uuid>,
}

/// `POST /api/team/{id}/ratify` — `proposed → authoritative` (FR-453, FR-454).
///
/// **`AdminUser`, so the authorization is the parameter list.** An agent, even
/// running with an admin's token in its environment, has no tool action shaped
/// like ratification (FR-455, FR-515); this route is what `cairn team ratify`
/// calls, and a member reaching it is refused by the extractor before the
/// handler exists.
///
/// **One statement decides it.** The `UPDATE` carries its own
/// `WHERE state = 'proposed'`, so two concurrent ratifications of the same
/// proposal race inside PostgreSQL rather than in this handler: exactly one
/// affects a row, and the loser affects zero (FR-454, SC-415). A read followed
/// by a write would have a window between them, and the loser's write would
/// silently reapply on top of the winner's. The state is read **only after** a
/// zero-row result, and only to name it in the refusal.
pub async fn ratify_team(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
    body: Option<Json<RatifyBody>>,
) -> ApiResult<Json<Value>> {
    require_capability(state.schema_version, TEAM_CAPABILITY)?;
    let supersedes = body.map(|Json(b)| b.supersedes).unwrap_or_default();

    // The relation and the ratification commit together: an admin's
    // `supersedes` is part of the ratification decision, and a crash between
    // the two would leave authoritative guidance whose replacement of the old
    // entry is recorded nowhere.
    let mut tx = state.pool.begin().await?;
    // **The reply carries the revision this write produced** (FR-457). The
    // acting device applies a transition the server has already made and no
    // pulled page has carried it yet, so this reply is the only place the row's
    // new version exists — and the local guard that stops a page fetched
    // *before* this ratification from rolling the row back needs a version to
    // decline it with. `changed_at` cannot serve: it is `GREATEST` over
    // transaction-start timestamps and a concurrent retirement can leave it
    // unmoved (see [`TeamWireRow::revision`]).
    //
    // Nothing here assigns the revision. `team_knowledge_revision_bump` does,
    // at statement time, on every row write — so this `RETURNING` reads what
    // the trigger just set rather than restating a rule a future statement
    // could forget. `NULL` on a deployment below `TEAM_REVISION_SCHEMA`, which
    // has no column to return.
    let ratified: Option<(chrono::DateTime<chrono::Utc>, Option<i64>)> = sqlx::query_as(&format!(
        "UPDATE team_knowledge
            SET state = 'authoritative', ratified_by_user_id = $1, ratified_at = now()
          WHERE id = $2 AND state = 'proposed'
        RETURNING ratified_at, {}",
        revision_or_null(state.schema_version)
    ))
    .bind(admin.id())
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((ratified_at, revision)) = ratified else {
        return Err(state_refusal(&state.pool, id, TeamState::Proposed).await);
    };

    if let Some(superseded) = supersedes {
        record_supersedes(&mut tx, id, superseded).await?;
    }
    tx.commit().await?;

    Ok(Json(json!({
        "id": id,
        "state": TeamState::Authoritative.as_str(),
        "ratified_by_user_id": admin.id(),
        "ratified_at": ratified_at.to_rfc3339(),
        "revision": revision,
        "supersedes": supersedes,
    })))
}

/// `POST /api/team/{id}/retire` — `authoritative → retired` (FR-456, FR-457).
///
/// **There is no route out of `retired`, and this is why there does not need to
/// be a check for one.** `WHERE state = 'authoritative'` refuses a retired row
/// exactly as it refuses a still-proposed one, and [`ratify_team`]'s
/// `WHERE state = 'proposed'` refuses a retired row in the other direction. So
/// "un-retire" is not a statement this server can execute (FR-465): restoring
/// retired guidance is a new proposal, which is itself ratified, so every
/// authoritative entry has exactly one ratification event in its own history.
///
/// Records **who as well as when** (FR-457). A timestamp alone does not say who
/// acted, and retirement is the transition most worth attributing: it withdraws
/// guidance from every account on the server, including accounts in projects the
/// retiring admin has never seen.
///
/// `content` is untouched, here and in the CAS above — a retired entry keeps
/// saying what it said (FR-461). The `UPDATE` has no clause capable of writing
/// it.
pub async fn retire_team(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    require_capability(state.schema_version, TEAM_CAPABILITY)?;
    // As `ratify_team`: the revision this write produced is in the reply and
    // nowhere else yet, and it is what the acting device's local guard declines
    // a stale page with. `retired_at` is exactly the timestamp that can fail to
    // move `changed_at`, which is what made this reply's version unusable
    // before the revision existed.
    let retired: Option<(chrono::DateTime<chrono::Utc>, Option<i64>)> = sqlx::query_as(&format!(
        "UPDATE team_knowledge
            SET state = 'retired', retired_at = now(), retired_by_user_id = $1
          WHERE id = $2 AND state = 'authoritative'
        RETURNING retired_at, {}",
        revision_or_null(state.schema_version)
    ))
    .bind(admin.id())
    .bind(id)
    .fetch_optional(&state.pool)
    .await?;
    let Some((retired_at, revision)) = retired else {
        return Err(state_refusal(&state.pool, id, TeamState::Authoritative).await);
    };

    Ok(Json(json!({
        "id": id,
        "state": TeamState::Retired.as_str(),
        "retired_by_user_id": admin.id(),
        "retired_at": retired_at.to_rfc3339(),
        "revision": revision,
    })))
}

/// The explicit `supersedes` edge a ratifying admin recorded (D431, FR-462).
///
/// Refuses an id that names nothing, and refuses the entry superseding itself.
/// The table carries no foreign key — deliberately, so a relation arriving
/// before the row it names is held for replay rather than dropped — which means
/// nothing but this check stands between a typo'd UUID and an edge pointing at
/// an entry that will never exist. An admin naming an entry that is not there is
/// far more likely to have mistyped it than to be describing a future arrival.
///
/// The target's *state* is deliberately not constrained beyond existing. The
/// edge records what an admin decided at ratification time, and the entry it
/// names can be retired afterwards by someone else — a rule that read the
/// target's state now would make the same decision valid or invalid depending
/// on when it was made, and would not stop the state changing a moment later
/// anyway.
async fn record_supersedes(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    superseded: Uuid,
) -> ApiResult<()> {
    if superseded == id {
        return Err(ApiError::invalid(
            "a team knowledge entry cannot supersede itself",
        ));
    }
    // `SELECT id`, not `SELECT 1`. Postgres types the literal `1` as `INT4`, and
    // decoding it into `i64` fails at runtime with a type-mismatch error — which
    // surfaced as an internal error on every single `--supersedes`, because no
    // test reached this line until the supersession semantics were wired to a
    // read path. Selecting the column avoids inventing a type for a constant.
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM team_knowledge WHERE id = $1")
        .bind(superseded)
        .fetch_optional(&mut **tx)
        .await?;
    if exists.is_none() {
        return Err(ApiError::not_found(
            "`supersedes` names no team knowledge entry on this server",
        ));
    }
    // Idempotent: re-recording the same edge is the same decision, and a
    // primary-key violation over it would refuse a ratification that had
    // already succeeded.
    sqlx::query(
        "INSERT INTO team_knowledge_relations (from_id, to_id, kind, basis)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(superseded)
    .bind(RelationKind::Supersedes.as_str())
    .bind(RelationBasis::ExplicitUser.as_str())
    .execute(&mut **tx)
    .await?;

    // **And the pointer**, in the same transaction as the ratification that
    // decided it (FR-462, D431). The relation records that an administrator made
    // the decision; `superseded_by_id` is what a reader consults and what
    // crosses the wire, so this is how a second device learns the old guidance
    // was replaced — the relation tables do not synchronize (see T104). A
    // ratification that wrote only the relation left `--supersedes` inert: every
    // canonical read still returned the replaced entry as competing guidance.
    //
    // Guarded on the target still being authoritative and not already
    // superseded, so this can neither resurrect a retired entry nor overwrite an
    // earlier supersession.
    sqlx::query(
        "UPDATE team_knowledge SET superseded_by_id = $1, superseded_at = now()
          WHERE id = $2 AND state = 'authoritative' AND superseded_by_id IS NULL",
    )
    .bind(id)
    .bind(superseded)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The refusal a lost compare-and-swap produces, naming the state the entry is
/// actually in (FR-454, FR-456, SC-415).
///
/// Read **after** the failed `UPDATE`, so this read decides nothing — it only
/// describes. Naming the actual state is what makes a concurrent transition
/// visible instead of clobbered: "not proposed" tells a caller to look, while
/// "at state retired" tells them what happened.
///
/// The message is word for word `cairn_store::global::state_conflict`'s, so an
/// operator sees the same sentence whether the local store or the server
/// refused. The state also travels as its own field, because a caller deciding
/// what to do next should not have to parse prose to find the one fact the
/// decision turns on.
async fn state_refusal(pool: &PgPool, id: Uuid, required: TeamState) -> ApiError {
    let actual: Result<Option<String>, _> =
        sqlx::query_scalar("SELECT state FROM team_knowledge WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await;
    let actual = match actual {
        Ok(Some(state)) => state,
        // A missing row is not a lost race. Said plainly, because "at state
        // none" would be a lie and a `409` would send a caller looking for a
        // concurrent admin who does not exist.
        Ok(None) => return ApiError::not_found(format!("no team knowledge entry {id}")),
        Err(e) => return e.into(),
    };
    ApiError::new(
        StatusCode::CONFLICT,
        "state_conflict",
        format!("team knowledge is at state {actual}, not the state this request required"),
    )
    .with_detail(json!({ "state": actual, "expected_state": required.as_str() }))
}

fn text(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn opt_text(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn opt_uuid(payload: &Value, key: &str) -> Option<Uuid> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn opt_time(payload: &Value, key: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(tokens: &[&str]) -> Vec<ProjectIdentity> {
        tokens
            .iter()
            .map(|t| ProjectIdentity(t.to_string()))
            .collect()
    }

    fn parsed(remote: &str) -> Vec<String> {
        remote_identities(remote).into_iter().map(|i| i.0).collect()
    }

    /// The three supported shapes agree about the same repository, and none of
    /// them contributes its own syntax.
    ///
    /// **Falsified by** splitting the host on `.`, keeping the scheme, or
    /// keeping the SSH username.
    #[test]
    fn every_remote_shape_yields_the_host_the_organisation_and_the_repository() {
        for remote in [
            "git@github.com:acme/widgets.git",
            "https://github.com/acme/widgets.git",
            "ssh://git@github.com/acme/widgets.git",
            "https://github.com/acme/widgets",
        ] {
            assert_eq!(
                parsed(remote),
                vec!["github.com", "acme", "widgets"],
                "{remote} did not reduce to host, organisation and repository"
            );
        }
    }

    /// Nested groups are each an identity, so a subgroup cannot be named freely.
    #[test]
    fn a_nested_path_contributes_every_group() {
        assert_eq!(
            parsed("ssh://git@gitlab.com/group/subgroup/repo.git"),
            vec!["gitlab.com", "group", "subgroup", "repo"]
        );
    }

    /// **Position decides, not spelling.** Here `com` is the organisation and
    /// `net` the repository, and both name the project.
    ///
    /// **Falsified by** filtering path components against a list of structural
    /// words, which is how the ingest side used to avoid the TLD — and which
    /// would leave this project's own organisation unscreened.
    #[test]
    fn a_path_component_spelled_like_syntax_is_kept() {
        assert_eq!(
            parsed("https://github.com/com/net.git"),
            vec!["github.com", "com", "net"]
        );
        assert_eq!(
            parsed("git@github.com:git/ssh.git"),
            vec!["github.com", "git", "ssh"]
        );
    }

    /// The `.git` suffix comes off once, so a repository named `git` survives.
    ///
    /// **Falsified by** `trim_end_matches`, which strips repeatedly and left
    /// this repository with no identity at all.
    #[test]
    fn the_git_suffix_is_removed_once_and_only_from_the_last_segment() {
        assert_eq!(
            parsed("git@github.com:acme/git.git"),
            vec!["github.com", "acme", "git"]
        );
        assert_eq!(
            parsed("https://github.com/my.git/widgets.git"),
            vec!["github.com", "my.git", "widgets"]
        );
    }

    /// A port is structure in a URL and a path in SCP form.
    ///
    /// `host:1234/repo.git` is the SCP spelling of a repository under a
    /// directory named `1234`; reading it as a port would drop an identity.
    #[test]
    fn a_port_is_stripped_only_where_a_port_is_meaningful() {
        assert_eq!(
            parsed("ssh://git@gitlab.com:2222/group/repo.git"),
            vec!["gitlab.com", "group", "repo"]
        );
        assert_eq!(
            parsed("git@gitlab.com:1234/repo.git"),
            vec!["gitlab.com", "1234", "repo"]
        );
    }

    /// Credentials are not identity, and a password containing `@` cannot hide
    /// the host behind it.
    #[test]
    fn credentials_are_not_identity() {
        assert_eq!(
            parsed("https://user:p@ss@github.com/acme/widgets.git"),
            vec!["github.com", "acme", "widgets"]
        );
    }

    /// A remote with no host still contributes its path, and names no host.
    #[test]
    fn a_local_path_remote_has_no_host_and_keeps_its_segments() {
        assert_eq!(
            parsed("/srv/git/widgets.git"),
            vec!["srv", "git", "widgets"]
        );
    }

    /// The ingest screen refuses what the client should have refused, using the
    /// same nine classes — never a second implementation (FR-579).
    #[test]
    fn ingest_refuses_the_same_classes_the_client_would_have() {
        let none: [ProjectIdentity; 0] = [];
        let leaking = json!({ "content": "Scratch files live at /Users/alice/tmp" });
        assert_eq!(
            screen_global_item(&leaking, &none).unwrap_err().class,
            "absolute_path"
        );
        let clean = json!({ "content": "Prefer thiserror over hand-rolled Display impls" });
        assert!(screen_global_item(&clean, &none).is_ok());
    }

    /// The case a client-side check structurally cannot catch: content naming
    /// project X, pushed by a client that was working in project Y.
    #[test]
    fn ingest_screens_against_every_project_the_pusher_belongs_to() {
        let membership = ids(&["project-x", "project-y"]);
        let naming_the_other = json!({ "content": "the project-x CI is slow" });
        assert_eq!(
            screen_global_item(&naming_the_other, &membership)
                .unwrap_err()
                .class,
            "project_identifying"
        );
        // A client working in project-y would have screened only against
        // project-y and let this through.
        let client_side_only = ids(&["project-y"]);
        assert!(screen_global_item(&naming_the_other, &client_side_only).is_ok());
    }

    /// An applicability value is screened as content here too (FR-578).
    #[test]
    fn ingest_screens_applicability_values() {
        let membership = ids(&["acme_internal"]);
        let payload = json!({
            "content": "harmless",
            "applicability": [{ "kind": "tool", "value": "acme_internal" }],
        });
        assert_eq!(
            screen_global_item(&payload, &membership).unwrap_err().class,
            "project_identifying"
        );
    }

    /// A refusal carries a class and nothing else (FR-547).
    #[test]
    fn an_ingest_refusal_never_carries_the_content() {
        let none: [ProjectIdentity; 0] = [];
        let payload = json!({ "content": "secret path /Users/alice/private" });
        let refusal = screen_global_item(&payload, &none).unwrap_err();
        let rendered = format!("{refusal:?}") + &format!("{:?}", refusal.into_api_error().message);
        assert!(!rendered.contains("alice"), "{rendered}");
        assert!(!rendered.contains("private"), "{rendered}");
    }

    /// The read-back half of T101: `writer_id` and `writer_seq` travel on the
    /// wire exactly as `upsert_personal` wrote them, so a peer can detect a
    /// gap in this writer's own stream (§9, FR-582).
    #[test]
    fn personal_row_json_carries_writer_id_and_writer_seq() {
        let now = chrono::Utc::now();
        let id = Uuid::now_v7();
        let v = personal_row_json(
            id,
            "fact",
            "prefer thiserror",
            Some("errors"),
            Some("thiserror"),
            "a-writer-id",
            7,
            now,
            None,
            None,
            &[],
        );
        assert_eq!(v["id"], id.to_string());
        assert_eq!(v["writer_id"], "a-writer-id");
        assert_eq!(v["writer_seq"], 7);
        assert_eq!(v["created_at"], now.to_rfc3339());
    }

    /// A tombstoned or superseded row still carries those fields on the wire —
    /// read-back is not limited to freshly proposed rows.
    #[test]
    fn personal_row_json_carries_supersession_and_forgetting() {
        let now = chrono::Utc::now();
        let successor = Uuid::now_v7();
        let v = personal_row_json(
            Uuid::now_v7(),
            "fact",
            "an old claim",
            None,
            None,
            "writer",
            1,
            now,
            Some(successor),
            Some(now),
            &[],
        );
        assert_eq!(v["superseded_by_id"], successor.to_string());
        assert_eq!(v["forgotten_at"], now.to_rfc3339());
    }

    /// Applicability rides along on each row, not as a separate array the
    /// caller has to correlate back by id.
    #[test]
    fn personal_row_json_carries_its_applicability() {
        let facts = vec![json!({ "kind": "language", "value": "rust" })];
        let v = personal_row_json(
            Uuid::now_v7(),
            "convention",
            "use rustfmt defaults",
            None,
            None,
            "writer",
            1,
            chrono::Utc::now(),
            None,
            None,
            &facts,
        );
        assert_eq!(v["applicability"], json!(facts));
    }

    /// The ingest path stores the facts it screened, and an absent
    /// `applicability` key is **not** an empty set.
    ///
    /// The distinction is the whole safety property: an entry with no facts
    /// applies to every project (D411, FR-435), so reading "the sender said
    /// nothing" as "the sender said everywhere" silently widens a record's
    /// audience on a lifecycle-only push.
    #[test]
    fn an_absent_applicability_key_asserts_nothing_and_an_empty_array_asserts_everywhere() {
        let lifecycle_only = json!({ "content": "unchanged", "forgotten_at": null });
        assert!(matches!(
            asserted_applicability(&lifecycle_only),
            Ok(Asserted::Nothing)
        ));
        let universal = json!({ "content": "x", "applicability": [] });
        match asserted_applicability(&universal) {
            Ok(Asserted::Facts(facts)) => assert!(facts.is_empty()),
            other => panic!(
                "an explicit empty array is an assertion: {:?}",
                other.is_ok()
            ),
        }
    }

    /// A kind outside `language | tool` is refused by name rather than handed to
    /// the `kind` CHECK, which would come back as a `500` and be retried
    /// forever.
    #[test]
    fn an_applicability_kind_outside_the_vocabulary_is_refused_by_name() {
        let payload = json!({
            "content": "x",
            "applicability": [
                { "kind": "language", "value": "rust" },
                { "kind": "topic", "value": "anything" },
            ],
        });
        assert_eq!(
            asserted_applicability(&payload).err().as_deref(),
            Some("topic")
        );
        let good = json!({
            "content": "x",
            "applicability": [{ "kind": "tool", "value": "docker" }],
        });
        match asserted_applicability(&good) {
            Ok(Asserted::Facts(facts)) => {
                assert_eq!(facts.len(), 1);
                assert_eq!(facts[0].kind, ApplicabilityKind::Tool);
                assert_eq!(facts[0].value, "docker");
            }
            other => panic!("a valid vocabulary was refused: {:?}", other.is_ok()),
        }
    }

    /// The refused kind is a vocabulary name and never the value beside it —
    /// the same discipline [`IngestRefusal`] keeps about content.
    #[test]
    fn a_refused_kind_never_carries_the_value_beside_it() {
        let payload = json!({
            "content": "x",
            "applicability": [{ "kind": "topic", "value": "acme_internal" }],
        });
        let rendered = asserted_applicability(&payload).err().unwrap();
        assert!(!rendered.contains("acme_internal"), "{rendered}");
    }

    fn a_team_row() -> TeamWireRow {
        TeamWireRow {
            id: Uuid::now_v7(),
            knowledge_type: "convention".to_string(),
            content: "we squash on merge".to_string(),
            topic_key: Some("merge_strategy".to_string()),
            value_key: Some("squash".to_string()),
            state: "authoritative".to_string(),
            proposed_by_user_id: Uuid::now_v7(),
            ratified_by_user_id: Some(Uuid::now_v7()),
            ratified_at: Some(chrono::Utc::now()),
            writer_id: Uuid::now_v7().to_string(),
            writer_seq: 4,
            created_at: chrono::Utc::now(),
            superseded_by_id: None,
            retired_by_user_id: None,
            retired_at: None,
            changed_at: chrono::Utc::now(),
            revision: Some(37),
            applicability: vec![json!({ "kind": "tool", "value": "git" })],
        }
    }

    /// **The wire shape is `cairn_store::global::SyncedTeamKnowledge`, field for
    /// field** (`crates/cairn-store/src/global.rs`) — `changed_at` excepted,
    /// which the mirror names `server_changed_at`.
    ///
    /// Asserted as an exact key set rather than field by field, because both
    /// directions of drift break the mirror: a missing field fails
    /// deserialization outright, and an extra one is how `origin_digest` would
    /// reach a wire it must never touch (D434, FR-551). The list is spelled out
    /// here because this crate does not depend on `cairn-store` — a change to
    /// the mirror's fields has to be made here too, and this test is what says
    /// so out loud.
    #[test]
    fn a_team_row_carries_exactly_the_local_mirrors_fields() {
        let expected: std::collections::BTreeSet<&str> = [
            "id",
            "knowledge_type",
            "content",
            "topic_key",
            "value_key",
            "applicability",
            "state",
            "proposed_by_user_id",
            "ratified_by_user_id",
            "ratified_at",
            "writer_id",
            "writer_seq",
            "created_at",
            "superseded_by_id",
            // Both halves of the retirement, for the same reason both halves of
            // the ratification are here: FR-457 asks who acted *and* when, and
            // this list is where the two schemas are held to agreeing. It sat
            // without `retired_by_user_id` for as long as the wire row did, so
            // the parity it asserts was parity on the wrong set — the test
            // passed and the field reached nobody.
            "retired_by_user_id",
            "retired_at",
            // The server's own version of the row (FR-457, FR-712a). The one
            // name in this list that is *not* spelled the same on the mirror:
            // it is stored there as `server_changed_at`, because on a device
            // the salient fact about the value is whose clock it came from.
            // Without it a device cannot tell a page fetched before its own
            // local transition from one fetched after, and the merge that
            // guesses wrong erases who acted.
            "changed_at",
            // And the monotonic version that actually orders two pages
            // (FR-456, FR-457, FR-465), stored on the mirror as
            // `server_revision` — the second name in this list spelled
            // differently there, for the same reason.
            //
            // `changed_at` stayed because it is still provenance and still what
            // a mirror below local schema 12 compares with; it is here
            // *alongside* this one, never instead of it. It could not order
            // anything on its own: it is `GREATEST` over columns stamped with
            // transaction-start `now()`, so a retirement can leave it byte-for-
            // byte where the preceding ratification left it, and two different
            // states then share one version.
            "revision",
        ]
        .into_iter()
        .collect();
        let wire = a_team_row().to_json();
        let actual: std::collections::BTreeSet<&str> = wire
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert_eq!(actual, expected);
        assert!(
            !wire.as_object().unwrap().contains_key("origin_digest"),
            "the origin digest is local-only and must never reach the wire"
        );
    }

    /// `writer_id` and `writer_seq` travel on a team row too (§9, FR-582), and
    /// `writer_id` stays parseable as the UUID the mirror types it as.
    #[test]
    fn a_team_row_carries_writer_identity_the_mirror_can_parse() {
        let row = a_team_row();
        let wire = row.to_json();
        assert_eq!(wire["writer_seq"], 4);
        let carried = wire["writer_id"].as_str().unwrap();
        assert_eq!(carried, row.writer_id);
        assert!(Uuid::parse_str(carried).is_ok(), "{carried}");
    }

    /// A retired entry is transmitted with its retirement on it. A device
    /// holding the authoritative copy learns the guidance was withdrawn from
    /// this row and nowhere else (FR-456, FR-457).
    #[test]
    fn a_retired_team_row_carries_its_retirement() {
        let retired_at = chrono::Utc::now();
        let successor = Uuid::now_v7();
        let mut row = a_team_row();
        row.state = "retired".to_string();
        row.retired_at = Some(retired_at);
        row.superseded_by_id = Some(successor);
        let wire = row.to_json();
        assert_eq!(wire["state"], "retired");
        assert_eq!(wire["retired_at"], retired_at.to_rfc3339());
        assert_eq!(wire["superseded_by_id"], successor.to_string());
    }

    /// A proposal that has never been ratified carries nulls rather than
    /// invented values — `ratified_at` present on a `proposed` row would be a
    /// ratification the mirror would then believe in.
    #[test]
    fn an_unratified_team_row_carries_nulls_not_defaults() {
        let mut row = a_team_row();
        row.state = "proposed".to_string();
        row.ratified_by_user_id = None;
        row.ratified_at = None;
        let wire = row.to_json();
        assert!(wire["ratified_by_user_id"].is_null());
        assert!(wire["ratified_at"].is_null());
    }

    /// The two names these routes refuse by are the two names
    /// `GET /api/version` advertises, so a client is told it lacks the same
    /// string it would have looked for.
    #[test]
    fn the_refusal_names_the_capability_the_version_route_advertises() {
        let at_three = crate::version::capabilities_for(3);
        assert!(at_three.contains(&PERSONAL_CAPABILITY), "{at_three:?}");
        assert!(at_three.contains(&TEAM_CAPABILITY), "{at_three:?}");
        let at_two = crate::version::capabilities_for(2);
        assert!(!at_two.contains(&PERSONAL_CAPABILITY), "{at_two:?}");
        assert!(!at_two.contains(&TEAM_CAPABILITY), "{at_two:?}");
    }

    /// A held-back deployment refuses both read-backs cleanly and by name,
    /// rather than reaching a table its migration has not created.
    #[test]
    fn below_schema_three_the_read_backs_refuse_by_name() {
        for capability in [PERSONAL_CAPABILITY, TEAM_CAPABILITY] {
            let refusal = require_capability(2, capability).unwrap_err();
            assert_eq!(refusal.status, StatusCode::CONFLICT);
            assert_eq!(refusal.code, "schema_too_old");
            assert!(refusal.message.contains(capability), "{}", refusal.message);
            assert!(require_capability(3, capability).is_ok());
        }
    }

    /// A cursor that cannot be parsed resumes from the start rather than from
    /// `now()`, so the failure mode is re-delivery into idempotent importers
    /// instead of a silently skipped window.
    #[test]
    fn an_unreadable_cursor_resumes_from_the_start() {
        let start = PageCursor::start();
        assert_eq!(PageCursor::decode(None), start);
        assert_eq!(PageCursor::decode(Some("last tuesday")), start);
        assert_eq!(
            PageCursor::decode(Some("2026-08-01T00:00:00Z|not-a-uuid")).id,
            Uuid::nil()
        );
    }

    /// The cursor round-trips both halves, and a bare timestamp still parses.
    ///
    /// A bare timestamp is what a build without the id half wrote. It resumes as
    /// `(that instant, nil)`, which re-delivers the rows at exactly that instant
    /// once — free, because every importer is idempotent by id, where a skip
    /// would not be. That asymmetry is why this is the one lenient parse in the
    /// pagination path.
    #[test]
    fn the_page_cursor_round_trips_and_accepts_the_older_form() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-08-01T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let id = Uuid::now_v7();
        let cursor = PageCursor { at, id };
        assert_eq!(PageCursor::decode(Some(&cursor.encode())), cursor);

        let legacy = PageCursor::decode(Some("2026-08-01T12:00:00Z"));
        assert_eq!(legacy.at, at);
        assert_eq!(
            legacy.id,
            Uuid::nil(),
            "a cursor with no id half must resume at the start of its instant, \
             not past it"
        );
    }

    /// The page size is the server's, not the caller's.
    #[test]
    fn a_requested_page_size_is_clamped_and_never_exceeds_the_servers() {
        let ask = |limit: Option<i64>| GlobalChangesQuery { since: None, limit }.page();
        assert_eq!(ask(None), crate::sync::PAGE);
        assert_eq!(ask(Some(1)), 1);
        assert_eq!(ask(Some(0)), 1);
        assert_eq!(ask(Some(-5)), 1);
        assert_eq!(ask(Some(crate::sync::PAGE + 1_000)), crate::sync::PAGE);
    }

    /// A remote yields its host, organisation and repository parts, because
    /// content naming any one of them names the project.
    #[test]
    fn a_remote_contributes_each_of_its_parts() {
        let tokens = parsed("git@github.com:acme/widgets.git");
        assert!(tokens.contains(&"github.com".to_string()), "{tokens:?}");
        assert!(tokens.contains(&"acme".to_string()), "{tokens:?}");
        assert!(tokens.contains(&"widgets".to_string()), "{tokens:?}");
        // Absent because it is the SSH username here — structure, not identity.
        // A repository *named* `git` is a different matter and is kept; see
        // `the_git_suffix_is_removed_once_and_only_from_the_last_segment`.
        assert!(!tokens.contains(&"git".to_string()), "{tokens:?}");
    }
}

#[cfg(test)]
mod team_ingest_tests {
    use super::*;

    /// Ingest screening is **uniform across domains** (T130).
    ///
    /// The same function, the same nine classes, the same identity union. A team
    /// item is not screened more loosely because it is destined for a
    /// server-wide default — if anything that content travels further, since an
    /// authoritative team entry reaches every account on the server regardless of
    /// project membership.
    #[test]
    fn team_ingest_is_screened_exactly_as_personal_ingest_is() {
        let membership = [ProjectIdentity("acme-widgets".to_string())];
        for content in [
            "Scratch files live at /Users/alice/tmp",
            "the acme-widgets CI is slow",
            "rm -rf ./target",
        ] {
            let payload = json!({ "content": content });
            let personal = screen_global_item(&payload, &membership);
            let team = screen_global_item(&payload, &membership);
            assert_eq!(
                personal.map_err(|e| e.class),
                team.map_err(|e| e.class),
                "{content:?} was screened differently for the two domains"
            );
            assert!(
                personal.is_err(),
                "{content:?} passed both domains' ingest screen"
            );
        }
    }
}
