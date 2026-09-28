//! Personal and team knowledge ingest and privacy policy.
//!
//! Server revalidates every pushed item: client validation is advisory, never a
//! privacy boundary.

use crate::auth::{AdminUser, SettledUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use cairn_core::domain::{RelationBasis, RelationKind, ServerRole, TeamState};
use cairn_core::validate::ProjectIdentity;
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

// ---------------------------------------------------------------------------
// Read API views
// ---------------------------------------------------------------------------

/// **Two routes, not one route with a namespace parameter.**
///
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

/// Server-owned bound for every ordinary web list.
pub(crate) fn view_page_limit(requested: Option<i64>) -> i64 {
    requested
        .unwrap_or(VIEW_PAGE_DEFAULT)
        .clamp(1, VIEW_PAGE_MAX)
}

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
        view_page_limit(self.limit)
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
/// Edge clients consume this stable server contract directly.
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
    /// `revision` is allocated from `team_revision_counter` by a trigger on
    /// every row write, and the allocation *is* a row lock: a writer holding a
    /// revision holds that lock until it commits or rolls back, so the next
    /// writer cannot take a number until the previous one is visible.
    ///
    /// That is stronger than monotonic, and the difference is the whole reason
    /// it is not a sequence. `nextval` is monotonic too, but it hands out
    /// numbers at *statement* time while rows become visible at *commit* — so a
    /// writer holding revision 100 uncommitted while another takes 101 and
    /// commits lets a pull see 101, advance its cursor, and never see 100 once
    /// it lands. Reproduced against a real database before the repair. What the
    /// feed needs is that a client cannot advance past a change that becomes
    /// visible later, and only commit-ordered allocation gives it.
    ///
    /// A rolled-back write returns its number rather than burning it, so there
    /// are no gaps — and a gap would otherwise be indistinguishable from a row
    /// that has not committed yet.
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
/// The message is stable for edge clients. The state also travels as its own
/// field, because a caller deciding
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
