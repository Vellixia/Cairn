//! Verification reports, and the summaries derived from them
//! (`contracts/verification-summary.md`, FR-811a–FR-811i, SC-765–SC-767).
//!
//! # The one idea
//!
//! **Authority reflects the execution boundary, not the HTTP route.** A route is
//! caller-selected input. Generic bearer authentication establishes *who is
//! reporting*; it establishes nothing whatever about *what ran*. Choosing a path
//! is therefore exactly as strong as choosing a discriminator field — which is
//! to say, not strong at all — and a module that let `/runs` mean more than
//! `/attestations` would be letting a URL manufacture provenance.
//!
//! So every client-reported result is [`REPORTED_AUTHORITY`], and the two routes
//! differ only in what payload they accept. That difference describes the
//! *reported check shape*; it is not independently established execution
//! provenance, and it changes neither route's authority.
//!
//! `cairn` is unreachable over HTTP at all. There is no code path from a request
//! to it, and [`assign_authority`] is the only place authority is decided.
//!
//! `remote_cairn` remains an enum value with **no producer in baseline Feature
//! 005**. It may be produced only once a future specification names a trusted
//! evidence mechanism that establishes Cairn-client execution and states what it
//! proves. Nothing speculative is built here to preserve the value.
//!
//! # What a report may say, and what it may not
//!
//! A report carries the reference, a verdict, a verifier kind and when the check
//! ran. That is all. No observed value, no source locator, no digest of file
//! content, no command output, no local path — those are evidence, they are
//! local-only under FR-707, and there is no server table for them.
//!
//! `authority` and `report_id` are **refused rather than ignored**. Silently
//! dropping a field a caller supplied lets them believe it was honoured: a client
//! whose `report_id` vanished would go on trying to address a report by an id the
//! server never used, and one whose `authority` vanished would believe it had
//! established something.
//!
//! # Derivation, not assertion
//!
//! The server never stores an asserted state. `verification` is re-derived from
//! the accepted report history in the same transaction as the report that caused
//! it, so a reader can never see a report without the state it produced.

use crate::auth::{require_member, SettledUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::State;
use axum::Json;
use cairn_core::domain::{KnowledgeDomain, VerifierKind};
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use std::str::FromStr;
use uuid::Uuid;

/// The authority every HTTP-reported result is assigned (§4).
///
/// A constant rather than a parameter, because a parameter is a place for a
/// caller's choice to arrive. Both routes read this same value; there is no
/// argument, no branch and no configuration that can make one of them stronger.
const REPORTED_AUTHORITY: &str = "remote_attested";

/// Fields a report may never carry, at any depth.
///
/// Two kinds, and both are refused for the same reason. `authority` and
/// `report_id` are server-assigned, so naming one is asking to decide something
/// that is not the caller's to decide. The rest are **evidence** — the observed
/// value, where it was observed, and a digest of what was seen — which is
/// local-only under FR-707 and has no server table to land in. A payload
/// carrying any of them is refused rather than stripped.
const REFUSED_FIELDS: &[&str] = &[
    "authority",
    "verification_authority",
    "report_id",
    "observed_value",
    "source_locator",
    "content_digest",
    "content_norm_digest",
    "value_digest",
    "digest",
    "fingerprint",
    "evidence",
    "evidence_facts",
    "evidence_id",
    "observations",
    "command",
    "command_output",
    "output",
    "stdout",
    "stderr",
    "exit_code",
    "path",
    "file_path",
    "absolute_path",
    "relevant_paths",
    // Free text is evidence wearing a friendly name. A `detail` or a `summary`
    // is where an observed value goes when somebody wanted to be helpful, and
    // the local run row already has both — they stay there.
    "detail",
    "details",
    "summary",
    // Derived state is the server's, exactly as it is for a knowledge command.
    "verification",
    "verification_basis",
    "evidence_fact_count",
    "last_verified_at",
];

/// The three verdicts a report may carry (§6's table columns).
const VERDICTS: &[&str] = &["passed", "failed", "inconclusive"];

/// Refuse a payload that names anything the caller may not decide.
///
/// Walks the whole body rather than checking the top level, because a refused
/// name nested inside an object is the same assertion wearing a hat.
fn reject_refused_fields(body: &Value) -> ApiResult<()> {
    fn walk(value: &Value) -> Option<&'static str> {
        match value {
            Value::Object(map) => {
                for (key, nested) in map {
                    if let Some(found) = REFUSED_FIELDS.iter().find(|f| f.eq_ignore_ascii_case(key))
                    {
                        return Some(found);
                    }
                    if let Some(found) = walk(nested) {
                        return Some(found);
                    }
                }
                None
            }
            Value::Array(items) => items.iter().find_map(walk),
            _ => None,
        }
    }
    match walk(body) {
        // Named, not echoed. The refusal says which field was the problem and
        // never repeats its value, because the value may be the very local path
        // or command output the field exists to keep out.
        Some(field) => Err(ApiError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "authority_not_assertable",
            format!(
                "`{field}` is not a caller's to supply: authority and report identity are \
                 assigned by the server, and evidence stays on the machine that observed it"
            ),
        )),
        None => Ok(()),
    }
}

/// The complete logical reference a report is about.
///
/// **Both parts, always.** A knowledge reference without its domain is not a
/// reference: the same UUID can exist as a project memory, a personal note, a
/// team entry and a pattern at once, and a bare id names all four or none. A
/// pattern is the other shape — its own discriminator and a null domain slot —
/// and coercing one into a personal `KnowledgeRef` would file a pattern's
/// verification against a personal record that may not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reference {
    Knowledge { domain: KnowledgeDomain, id: Uuid },
    Pattern { pattern_id: Uuid },
}

impl Reference {
    /// Read a reference out of a request body.
    ///
    /// Accepts `memory_ref: { domain, knowledge_id }` — the contract's spelling —
    /// and the pattern form `memory_ref: { ref_kind: "pattern", knowledge_id }`.
    /// A body that names both a pattern kind and a domain is refused rather than
    /// resolved in either direction: it states two incompatible things, and
    /// picking one would be guessing which the caller meant.
    fn parse(body: &Value) -> ApiResult<Self> {
        let r = body
            .get("memory_ref")
            .or_else(|| body.get("reference"))
            .ok_or_else(|| ApiError::invalid("a report must name what it is about"))?;
        let named_pattern = r.get("pattern_id").is_some();
        let id = r
            .get("knowledge_id")
            .or_else(|| r.get("pattern_id"))
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| ApiError::invalid("`knowledge_id` is required and must be a uuid"))?;
        // **`pattern_id` is itself the discriminator.** `PatternRef(pattern_id)`
        // is the reference's whole shape (`data-model.md` §6.1), so a body that
        // names the field has already said which kind it means and should not
        // have to say it twice. A domain alongside it is still contradictory and
        // still refused below.
        let kind = r
            .get("ref_kind")
            .and_then(Value::as_str)
            .or(named_pattern.then_some("pattern"));
        let domain = r.get("domain").and_then(Value::as_str);

        match (kind, domain) {
            (Some("pattern"), Some(named)) => Err(ApiError::invalid(format!(
                "a pattern reference has no domain, and this one names `{named}`; a pattern \
                 is its own discriminator and its domain slot is null"
            ))),
            (Some("pattern"), None) => Ok(Reference::Pattern { pattern_id: id }),
            (Some("knowledge") | None, Some(named)) => {
                let domain = KnowledgeDomain::from_str(named)
                    .map_err(|_| ApiError::invalid(format!("`{named}` is not a domain")))?;
                Ok(Reference::Knowledge { domain, id })
            }
            (Some("knowledge") | None, None) => Err(ApiError::invalid(
                "a knowledge reference needs its domain: the same id can name a project \
                 memory, a personal note and a team entry at once",
            )),
            (Some(other), _) => Err(ApiError::invalid(format!("`{other}` is not a ref kind"))),
        }
    }

    fn ref_kind(self) -> &'static str {
        match self {
            Reference::Knowledge { .. } => "knowledge",
            Reference::Pattern { .. } => "pattern",
        }
    }

    fn domain(self) -> Option<&'static str> {
        match self {
            Reference::Knowledge { domain, .. } => Some(domain.as_str()),
            Reference::Pattern { .. } => None,
        }
    }

    fn id(self) -> Uuid {
        match self {
            Reference::Knowledge { id, .. } => id,
            Reference::Pattern { pattern_id } => pattern_id,
        }
    }

    /// The same string the database's generated column produces.
    fn key(self) -> String {
        match self {
            Reference::Knowledge { domain, id } => format!("knowledge:{}:{id}", domain.as_str()),
            Reference::Pattern { pattern_id } => format!("pattern:{pattern_id}"),
        }
    }
}

/// Where a report's subject lives, and who may report about it.
struct Binding {
    project_id: Option<Uuid>,
    owner_user_id: Option<Uuid>,
}

/// Resolve the reference to a record and check this caller may speak about it
/// (§7, §10 steps 2 and 3).
///
/// **A record that does not exist and a record the caller may not see give the
/// same answer**, for every domain. Distinguishing them would let anyone with an
/// account probe for ids: report about a guess, and the refusal says whether the
/// guess was real. That is the enumeration oracle FR-894a closes elsewhere, and
/// a verification route is no less of an oracle for being a write.
async fn resolve(pool: &PgPool, reference: Reference, user: Uuid) -> ApiResult<Binding> {
    let hidden = || ApiError::not_found("no such record");
    match reference {
        Reference::Knowledge {
            domain: KnowledgeDomain::Project,
            id,
        } => {
            let project_id: Uuid =
                sqlx::query_scalar("SELECT project_id FROM memories WHERE id = $1")
                    .bind(id)
                    .fetch_optional(pool)
                    .await?
                    .ok_or_else(hidden)?;
            // Membership decided here rather than by the extractor's own
            // refusal, because that one answers 403 and the whole point is that
            // both answers are the same.
            require_member(pool, project_id, user)
                .await
                .map_err(|_| hidden())?;
            Ok(Binding {
                project_id: Some(project_id),
                owner_user_id: None,
            })
        }
        Reference::Knowledge {
            domain: KnowledgeDomain::Personal,
            id,
        } => {
            let owner: Uuid = sqlx::query_scalar(
                "SELECT owner_user_id FROM personal_knowledge WHERE id = $1 AND forgotten_at IS NULL",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(hidden)?;
            // Personal knowledge is owner-only, so somebody else's record is
            // indistinguishable from one that is not there.
            if owner != user {
                return Err(hidden());
            }
            Ok(Binding {
                project_id: None,
                owner_user_id: Some(owner),
            })
        }
        Reference::Knowledge {
            domain: KnowledgeDomain::Team,
            id,
        } => {
            // Ratified team knowledge is server-global: any authenticated
            // account may report a check against guidance it is expected to
            // follow. A **proposed** entry is not — it reaches its author and
            // any administrator and nobody else (`sync-namespaces.md` §1a) — so
            // reporting about somebody else's proposal is refused with the same
            // `404` a record that does not exist gets. The alternative turns
            // this route into a way to discover which proposals exist, and to
            // attach verification history to one before its author has decided
            // whether to keep it.
            let row: Option<(String, Uuid)> = sqlx::query_as(
                "SELECT state, proposed_by_user_id FROM team_knowledge WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?;
            let (state, proposer) = row.ok_or_else(hidden)?;
            if state == "proposed" && proposer != user && !is_admin(pool, user).await? {
                return Err(hidden());
            }
            Ok(Binding {
                project_id: None,
                owner_user_id: None,
            })
        }
        Reference::Pattern { pattern_id } => {
            let owner: Uuid = sqlx::query_scalar(
                "SELECT owner_user_id FROM shared_patterns
                  WHERE pattern_id = $1 AND forgotten_at IS NULL",
            )
            .bind(pattern_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(hidden)?;
            if owner != user {
                return Err(hidden());
            }
            Ok(Binding {
                project_id: None,
                owner_user_id: Some(owner),
            })
        }
    }
}

/// Whether this account administers the server.
///
/// Read here rather than taken from the extractor's `SettledUser`, because the
/// only caller is the team branch above and threading a role through every
/// domain's resolution would put an authorization input in front of three
/// checks that must not consult it — a personal record is its owner's, and an
/// administrator is not its owner.
async fn is_admin(pool: &PgPool, user: Uuid) -> ApiResult<bool> {
    let role: Option<String> = sqlx::query_scalar("SELECT role::text FROM users WHERE id = $1")
        .bind(user)
        .fetch_optional(pool)
        .await?;
    Ok(role.as_deref() == Some("admin"))
}

/// The authority assigned to a report that arrived over HTTP.
///
/// **The only place authority is decided, and it takes no arguments** — not the
/// route, not the verifier kind, not the account, not the payload. That is the
/// whole design: a function with nothing to branch on cannot be made to branch
/// the wrong way, and every attempt to smuggle authority in has to get past a
/// signature that accepts nothing to smuggle it in.
///
/// A `verifier_kind` of `cairn` would be a name, not an execution. There is no
/// mapping from kind to authority here for exactly that reason.
fn assign_authority() -> &'static str {
    REPORTED_AUTHORITY
}

/// `POST /api/verification/runs` — a client report shaped as a deterministic run.
///
/// The name says "run" and the authority says `remote_attested`, and the gap
/// between those two is deliberate: an authenticated client *said* a check ran.
/// Believing the URL would be believing the caller about the one thing the
/// caller is not in a position to establish.
pub async fn report_run(
    state: State<AppState>,
    user: SettledUser,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    ingest(state, user, body, Attestation::NotApplicable).await
}

/// `POST /api/verification/attestations` — an agent attestation relayed by a client.
///
/// Requires and records the attesting agent, which `/runs` does not. That is a
/// difference in what the payload must describe, and it is **not** a difference
/// in what the report establishes: both are `remote_attested`, and this route is
/// not weaker than the other any more than the other is stronger than this one.
pub async fn report_attestation(
    state: State<AppState>,
    user: SettledUser,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    // **Read here, required inside.** This route does require it — §4 says an
    // attestation records the agent whose attestation it relays — but requiring
    // it *at this point* would judge the payload before deciding whether the
    // caller may see the record at all, and an outsider would learn from a
    // "malformed attestation" that the id they guessed was real. So the check
    // travels into `ingest` and runs after authorization, with every other
    // payload rule.
    //
    // `attesting_agent` and not `agent`: this payload already concerns a check,
    // and a bare `agent` beside a `verifier_kind` reads as the agent that *ran*
    // it, which is the one thing an attestation does not establish.
    let agent = body
        .get("attesting_agent")
        .and_then(Value::as_str)
        .map(str::to_string);
    ingest(state, user, body, Attestation::Relaying(agent)).await
}

/// Which route a report arrived on, and what it named.
///
/// A two-state type rather than an `Option<String>`, because `None` was doing
/// two jobs — "a run report, no attesting agent applies" and "an attestation
/// that failed to name one" — and only the second is an error.
enum Attestation {
    /// `/api/verification/runs`. No attesting agent applies.
    NotApplicable,
    /// `/api/verification/attestations`, carrying whatever it named.
    Relaying(Option<String>),
}

/// One report, from either route (§10's ordered checks).
async fn ingest(
    State(state): State<AppState>,
    user: SettledUser,
    body: Value,
    attesting: Attestation,
) -> ApiResult<Json<Value>> {
    // **§10's order, and the order is the point.**
    //
    // 2, 3 — resolve and authorize *before* the payload is judged. A caller who
    // may not see the record gets the same `404` whether their payload was
    // pristine or asserted authority, because the alternative leaks: a `400`
    // saying "authority is not yours to name" tells an outsider that the id they
    // guessed was real, and a refusal meant to protect the authority boundary
    // becomes an oracle against the existence boundary. Refusing a caller
    // everything about a record they cannot see includes refusing to grade their
    // request.
    let reference = Reference::parse(&body)?;
    let binding = resolve(&state.pool, reference, user.id()).await?;

    // 5 — only now, for a caller entitled to be here, is the payload judged.
    reject_refused_fields(&body)?;
    let attesting_agent = match attesting {
        Attestation::NotApplicable => None,
        Attestation::Relaying(Some(agent)) => Some(agent),
        Attestation::Relaying(None) => {
            return Err(ApiError::invalid(
                "an attestation names the agent whose attestation it relays; \
                 `attesting_agent` is required",
            ))
        }
    };

    // 4 — closed vocabularies.
    let verdict = body
        .get("verdict")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("`verdict` is required"))?;
    if !VERDICTS.contains(&verdict) {
        return Err(ApiError::invalid(format!("`{verdict}` is not a verdict")));
    }
    let verifier_kind = body
        .get("verifier_kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::invalid("`verifier_kind` is required"))?;
    // Validated against the declared vocabulary, which is also what stops
    // `verifier_kind: "cairn"` from being a back door: `cairn` is not a verifier
    // kind, so it is refused here, and even if it were the kind never reaches
    // `assign_authority`.
    VerifierKind::from_str(verifier_kind)
        .map_err(|_| ApiError::invalid(format!("`{verifier_kind}` is not a verifier kind")))?;
    let run_at: chrono::DateTime<chrono::Utc> = body
        .get("run_at")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
        .ok_or_else(|| ApiError::invalid("`run_at` is required and must be RFC 3339"))?;

    // 6 — the server assigns both.
    let report_id = Uuid::now_v7();
    let authority = assign_authority();

    let mut tx = state.pool.begin().await?;

    // 7 — insert, or discover this is a retry. The natural key is the complete
    // reference, the authenticated account, the verifier kind and `run_at`; the
    // account is in it so two people reporting the same team entry are two
    // reports rather than one overwriting the other.
    let inserted: Option<(Uuid,)> = sqlx::query_as(
        "INSERT INTO verification_reports
            (report_id, ref_kind, domain, knowledge_id, project_id, owner_user_id,
             account_id, verdict, verifier_kind, attesting_agent, authority, run_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (reference_key, account_id, verifier_kind, run_at) DO NOTHING
         RETURNING report_id",
    )
    .bind(report_id)
    .bind(reference.ref_kind())
    .bind(reference.domain())
    .bind(reference.id())
    .bind(binding.project_id)
    .bind(binding.owner_user_id)
    .bind(user.id())
    .bind(verdict)
    .bind(verifier_kind)
    .bind(attesting_agent.as_deref())
    .bind(authority)
    .bind(run_at)
    .fetch_optional(&mut *tx)
    .await?;

    if inserted.is_none() {
        // A repeat changes no state. That matters beyond politeness: §6 requires
        // a *second, subsequent* passed run to leave `conflicted`, and if a
        // resend counted, that guarantee would be satisfiable by pressing send
        // twice.
        let existing: (Uuid,) = sqlx::query_as(
            "SELECT report_id FROM verification_reports
              WHERE reference_key = $1 AND account_id = $2
                AND verifier_kind = $3 AND run_at = $4",
        )
        .bind(reference.key())
        .bind(user.id())
        .bind(verifier_kind)
        .bind(run_at)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(Json(json!({
            "report_id": existing.0,
            "applied": "duplicate",
            "authority": authority,
        })));
    }

    // 8 — re-derive, in this same transaction, so a reader can never see a
    // report without the state it produced.
    let summary = derive(&mut tx, reference).await?;
    tx.commit().await?;

    Ok(Json(json!({
        "report_id": report_id,
        "applied": "accepted",
        // Echoed so a caller can see what was assigned — and see that it is not
        // what they might have hoped to assign.
        "authority": authority,
        "verification": summary.state,
        "reference_key": reference.key(),
    })))
}

/// What a re-derivation concluded.
struct Summary {
    state: String,
}

/// Re-derive one record's verification from its accepted reports (§6).
///
/// **One derivation, two storage locations.** The project domain keeps using
/// `memories`' own five columns because they exist and are already populated;
/// everything else lands in `knowledge_verification`, keyed by the complete
/// reference so a personal and a team record sharing a UUID keep separate
/// summaries. Both are computed by this function from the same table, so there
/// is one rule and two places it is written — never two rules.
async fn derive(tx: &mut sqlx::PgConnection, reference: Reference) -> ApiResult<Summary> {
    let rows = sqlx::query(
        "SELECT verdict, verifier_kind, run_at
           FROM verification_reports
          WHERE reference_key = $1
          ORDER BY run_at, received_at",
    )
    .bind(reference.key())
    .fetch_all(&mut *tx)
    .await?;

    let mut state = "unverified".to_string();
    let mut basis: Vec<String> = Vec::new();
    let mut last_verified: Option<chrono::DateTime<chrono::Utc>> = None;
    for row in &rows {
        let verdict: String = row.get(0);
        let kind: String = row.get(1);
        let run_at: chrono::DateTime<chrono::Utc> = row.get(2);
        state = advance(&state, &verdict).to_string();
        if !basis.contains(&kind) {
            basis.push(kind);
        }
        if verdict == "passed" {
            last_verified = Some(run_at);
        }
    }
    // Counted from accepted reports, never from a number a client sent — there
    // is no field for one, and this is why.
    let count = rows.len() as i32;
    let authority: Option<&str> = (!rows.is_empty()).then_some(REPORTED_AUTHORITY);
    let basis_json = Value::Array(basis.into_iter().map(Value::String).collect());

    match reference {
        Reference::Knowledge {
            domain: KnowledgeDomain::Project,
            id,
        } => {
            sqlx::query(
                "UPDATE memories
                    SET verification = $2, verification_authority = $3,
                        verification_basis = $4, evidence_fact_count = $5,
                        last_verified_at = $6, updated_at = now()
                  WHERE id = $1",
            )
            .bind(id)
            .bind(&state)
            .bind(authority)
            .bind(&basis_json)
            .bind(count)
            .bind(last_verified)
            .execute(&mut *tx)
            .await?;
        }
        other => {
            sqlx::query(
                "INSERT INTO knowledge_verification
                    (ref_kind, domain, knowledge_id, verification, verification_authority,
                     verification_basis, evidence_fact_count, last_verified_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (reference_key) DO UPDATE
                    SET verification = EXCLUDED.verification,
                        verification_authority = EXCLUDED.verification_authority,
                        verification_basis = EXCLUDED.verification_basis,
                        evidence_fact_count = EXCLUDED.evidence_fact_count,
                        last_verified_at = EXCLUDED.last_verified_at",
            )
            .bind(other.ref_kind())
            .bind(other.domain())
            .bind(other.id())
            .bind(&state)
            .bind(authority)
            .bind(&basis_json)
            .bind(count)
            .bind(last_verified)
            .execute(&mut *tx)
            .await?;
        }
    }
    Ok(Summary { state })
}

/// §6's transition table, and nothing else.
///
/// Two entries are deliberate and mirror the shipped local machine
/// (`cairn-core/src/verify.rs`): a `failed` report against a `verified` record
/// yields `conflicted` rather than quietly demoting to `unverified`, because a
/// contradiction is information and losing it is how a record silently becomes
/// unremarkable; and the exit from `conflicted` lands on `needs_recheck`, never
/// straight back on `verified`, so a contradiction always costs one more
/// deliberate run.
fn advance(current: &str, verdict: &str) -> &'static str {
    match (current, verdict) {
        (_, "passed") if current == "conflicted" => "needs_recheck",
        (_, "passed") => "verified",
        ("verified", "failed") => "conflicted",
        ("unverified", "failed") => "unverified",
        ("needs_recheck", "failed") => "needs_recheck",
        ("drifted", "failed") => "drifted",
        ("conflicted", "failed") => "conflicted",
        // `inconclusive` is a result, not an error: the check ran and could
        // establish neither outcome, so the record keeps the state it had.
        ("verified", _) => "verified",
        ("needs_recheck", _) => "needs_recheck",
        ("drifted", _) => "drifted",
        ("conflicted", _) => "conflicted",
        _ => "unverified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every path through the table, including the two that are easy to get
    /// backwards.
    #[test]
    fn the_transition_table_matches_the_contract() {
        assert_eq!(advance("unverified", "passed"), "verified");
        assert_eq!(advance("unverified", "failed"), "unverified");
        assert_eq!(advance("unverified", "inconclusive"), "unverified");
        assert_eq!(advance("verified", "passed"), "verified");
        assert_eq!(advance("verified", "failed"), "conflicted");
        assert_eq!(advance("verified", "inconclusive"), "verified");
        assert_eq!(advance("needs_recheck", "passed"), "verified");
        assert_eq!(advance("needs_recheck", "failed"), "needs_recheck");
        assert_eq!(advance("drifted", "passed"), "verified");
        assert_eq!(advance("drifted", "failed"), "drifted");
        // The two that carry the reasoning.
        assert_eq!(
            advance("conflicted", "passed"),
            "needs_recheck",
            "a contradiction must cost one more deliberate run, not be cleared \
             by the next passing report"
        );
        assert_eq!(advance("conflicted", "failed"), "conflicted");
    }

    /// Authority has nothing to branch on, so nothing can branch it.
    #[test]
    fn authority_is_one_value_and_takes_no_input() {
        assert_eq!(assign_authority(), "remote_attested");
        assert_ne!(assign_authority(), "cairn");
        assert_ne!(assign_authority(), "remote_cairn");
    }

    #[test]
    fn a_refused_field_is_found_at_any_depth_and_named_without_its_value() {
        let body = json!({
            "memory_ref": { "domain": "project", "knowledge_id": Uuid::now_v7() },
            "nested": { "deeper": { "source_locator": "/Users/me/secret/path.rs" } }
        });
        let err = reject_refused_fields(&body).expect_err("a refused name must be caught");
        let rendered = format!("{err:?}");
        assert!(
            rendered.contains("source_locator"),
            "the refusal must say which field: {rendered}"
        );
        assert!(
            !rendered.contains("/Users/me"),
            "the refusal echoed the value it exists to keep out: {rendered}"
        );
    }

    #[test]
    fn a_knowledge_reference_without_a_domain_is_refused() {
        let body = json!({ "memory_ref": { "knowledge_id": Uuid::now_v7() } });
        assert!(
            Reference::parse(&body).is_err(),
            "a bare id names a project memory, a personal note and a team entry \
             at once, so it names none of them"
        );
    }

    #[test]
    fn a_pattern_cannot_be_given_a_domain() {
        let body = json!({
            "memory_ref": { "ref_kind": "pattern", "domain": "personal",
                            "knowledge_id": Uuid::now_v7() }
        });
        assert!(
            Reference::parse(&body).is_err(),
            "a pattern coerced into a personal KnowledgeRef would file its \
             verification against a personal record that may not exist"
        );
    }

    #[test]
    fn the_reference_key_matches_the_generated_column() {
        let id = Uuid::now_v7();
        assert_eq!(
            Reference::Knowledge {
                domain: KnowledgeDomain::Team,
                id
            }
            .key(),
            format!("knowledge:team:{id}")
        );
        assert_eq!(
            Reference::Pattern { pattern_id: id }.key(),
            format!("pattern:{id}")
        );
    }
}
