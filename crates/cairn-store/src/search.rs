//! Memory retrieval: exact filters, FTS5/BM25 relevance, scope-first ranking
//! (FR-022 – FR-026, D3).
//!
//! Scope dominates. A mediocre match about *this task* beats an excellent match
//! about an unrelated one — that is what makes Cairn's recall correct rather
//! than merely similar.

use crate::{repo, rows, Result, Store};
use cairn_core::domain::*;
use cairn_core::wire::{MemoryQuery, MemoryResult, Provenance, RankInfo};
use chrono::{DateTime, Utc};
use sqlx::Row;
use uuid::Uuid;

/// Where the search is being run from, so scope precedence can be applied.
#[derive(Debug, Clone, Default)]
pub struct SearchContext {
    pub branch: Option<String>,
    pub task_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
}

const DEFAULT_LIMIT: i64 = 10;
const MAX_LIMIT: i64 = 50;

/// Run a memory search.
pub async fn search(
    store: &Store,
    project_id: Uuid,
    q: &MemoryQuery,
    ctx: &SearchContext,
) -> Result<Vec<MemoryResult>> {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let state = q.state.unwrap_or(MemoryState::Active);

    let mut sql = String::from(
        "SELECT m.*, \
                CASE m.scope WHEN 'task' THEN 0 WHEN 'branch' THEN 1 \
                             WHEN 'project' THEN 2 ELSE 3 END AS scope_bucket",
    );
    if q.query.is_some() {
        sql.push_str(
            ", -bm25(memory_fts) AS relevance FROM memories m \
                      JOIN memory_fts ON memory_fts.rowid = m.rowid \
                      WHERE memory_fts MATCH ?",
        );
    } else {
        sql.push_str(", 0.0 AS relevance FROM memories m WHERE 1 = 1");
    }
    sql.push_str(" AND m.project_id = ? AND m.deleted_at IS NULL");
    if q.as_of.is_none() {
        sql.push_str(" AND m.state = ?");
    } else {
        // A historical answer is precisely the set of proposals that are no
        // longer current, so filtering to `active` would return the wrong
        // thing. The temporal predicate replaces the lifecycle one
        // (`contracts/knowledge.md` §Temporal queries).
        sql.push_str(
            " AND m.effective_from IS NOT NULL AND m.effective_from <= ?\
              AND (m.superseded_at IS NULL OR m.superseded_at > ?)",
        );
    }
    // A topic key is an identity, not text: it is matched by exact or prefix
    // SQL comparison and never by FTS (data-model.md §2.1).
    if let Some(topic) = &q.topic_key {
        if topic.ends_with('.') {
            sql.push_str(" AND m.topic_key LIKE ? ESCAPE '\\'");
        } else {
            sql.push_str(" AND m.topic_key = ?");
        }
    }
    if q.conflicted || q.corroborated {
        // A subject state is derived, so it cannot be a SQL predicate. What SQL
        // can do is narrow to the rows that could possibly qualify.
        sql.push_str(" AND m.topic_key IS NOT NULL");
    }

    // Explicit filters win; otherwise restrict to the scopes that apply here.
    let mut scope_clause = String::new();
    if let Some(scope) = q.scope {
        scope_clause.push_str(" AND m.scope = ?");
        if q.scope_key.is_some() {
            scope_clause.push_str(" AND m.scope_key = ?");
        }
        let _ = scope;
    } else if ctx.branch.is_some() || ctx.task_id.is_some() || ctx.session_id.is_some() {
        let mut parts = vec!["m.scope = 'project'".to_string()];
        if ctx.branch.is_some() {
            parts.push("(m.scope = 'branch' AND m.scope_key = ?)".into());
        }
        if ctx.task_id.is_some() {
            parts.push("(m.scope = 'task' AND m.scope_key = ?)".into());
        }
        if ctx.session_id.is_some() {
            parts.push("(m.scope = 'session' AND m.scope_key = ?)".into());
        }
        scope_clause.push_str(&format!(" AND ({})", parts.join(" OR ")));
    }
    sql.push_str(&scope_clause);

    // A `drifted` memory is still returned by default: hiding it would make an
    // agent silently re-derive knowledge Cairn holds (FR-373). These filters
    // narrow deliberately; they change no default.
    if q.verification.is_some() {
        sql.push_str(" AND m.verification = ?");
    }
    if q.authority.is_some() {
        sql.push_str(" AND m.verification_authority = ?");
    }

    if q.kind.is_some() {
        sql.push_str(" AND m.type = ?");
    }
    sql.push_str(" ORDER BY scope_bucket ASC, relevance DESC, m.created_at DESC LIMIT ?");

    let mut query = sqlx::query(&sql);
    if let Some(text) = &q.query {
        query = query.bind(fts_query(text));
    }
    query = query.bind(project_id.to_string());
    match &q.as_of {
        None => query = query.bind(state.as_str()),
        Some(t) => {
            let at = t.to_rfc3339();
            query = query.bind(at.clone()).bind(at);
        }
    }
    if let Some(topic) = &q.topic_key {
        if let Some(prefix) = topic.strip_suffix('.') {
            let escaped = prefix
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            query = query.bind(format!("{escaped}.%"));
        } else {
            query = query.bind(topic.clone());
        }
    }

    if let Some(scope) = q.scope {
        query = query.bind(scope.as_str());
        if let Some(key) = &q.scope_key {
            query = query.bind(key.clone());
        }
    } else {
        if let Some(b) = &ctx.branch {
            query = query.bind(b.clone());
        }
        if let Some(t) = ctx.task_id {
            query = query.bind(t.to_string());
        }
        if let Some(s) = ctx.session_id {
            query = query.bind(s.to_string());
        }
    }
    if let Some(v) = q.verification {
        query = query.bind(v.as_str());
    }
    if let Some(a) = q.authority {
        query = query.bind(a.as_str());
    }
    if let Some(kind) = q.kind {
        query = query.bind(kind.as_str());
    }
    // A derived filter has to see every candidate before the limit is applied,
    // or the limit would cut the set the derivation is computed over.
    let subject_filter = q.conflicted || q.corroborated;
    query = query.bind(if subject_filter {
        SUBJECT_FILTER_SCAN_MAX
    } else {
        limit
    });

    let raw = query.fetch_all(store.pool()).await?;
    let keep = if subject_filter {
        Some(qualifying_subjects(store, project_id, &raw, q.conflicted, q.corroborated).await?)
    } else {
        None
    };
    let now = Utc::now();

    // Which rows the caller will actually see, decided *before* anything is
    // derived. A subject filter reads up to `SUBJECT_FILTER_SCAN_MAX` rows to
    // find fifty; enriching as we go would derive five hundred subjects to
    // return ten.
    let mut kept: Vec<(&sqlx::sqlite::SqliteRow, Memory, Option<String>)> = Vec::new();
    for r in &raw {
        let m = rows::memory_bare(r)?;
        let topic_key: Option<String> = r.try_get("topic_key").unwrap_or(None);
        if let Some(keep) = &keep {
            match &topic_key {
                Some(t) if keep.contains_key(&(m.scope, m.scope_key.clone(), t.clone())) => {}
                _ => continue,
            }
        }
        if kept.len() >= limit as usize {
            break;
        }
        kept.push((r, m, topic_key));
    }

    // One derivation per distinct subject among the results, not one per
    // result: several members of one subject are the common case, and the
    // answer is the same for all of them. A filtered search has already derived
    // exactly these, and that work is reused rather than repeated.
    let mut subjects: std::collections::BTreeMap<
        (MemoryScope, String, String),
        crate::knowledge::SubjectRead,
    > = keep.unwrap_or_default();
    for (_, m, topic_key) in &kept {
        if let Some(topic) = topic_key {
            let key = (m.scope, m.scope_key.clone(), topic.clone());
            // `entry` would need the derivation eagerly, and deriving a subject
            // already in the map is the cost this cache exists to avoid.
            if let std::collections::btree_map::Entry::Vacant(slot) = subjects.entry(key) {
                slot.insert(
                    crate::knowledge::subject(
                        store,
                        project_id,
                        m.scope,
                        &m.scope_key,
                        topic,
                        crate::repo::DEFAULT_RECONCILE_MEMBERS_MAX,
                    )
                    .await?,
                );
            }
        }
    }

    let mut out = Vec::with_capacity(kept.len());
    for (r, m, topic_key) in kept {
        let evidence = repo::evidence_for(store, m.id).await?;
        let relevance: f64 = r.try_get("relevance").unwrap_or(0.0);
        let scope_bucket: i64 = r.try_get("scope_bucket").unwrap_or(3);
        out.push(MemoryResult {
            id: m.id,
            kind: m.kind,
            scope: m.scope,
            scope_key: m.scope_key.clone(),
            content: m.content.clone(),
            state: m.state,
            local_only: m.local_only,
            superseded_by_id: m.superseded_by_id,
            created_at: m.created_at,
            provenance: Provenance {
                session_id: m.origin_session_id,
                agent: repo::session(store, m.origin_session_id)
                    .await
                    .ok()
                    .map(|s| s.agent),
                observation_ids: evidence.iter().map(|e| e.observation_id).collect(),
                evidence_count: evidence.len(),
                deleted_observation_ids: evidence
                    .iter()
                    .filter(|e| e.deleted)
                    .map(|e| e.observation_id)
                    .collect(),
            },
            rank: RankInfo {
                scope_bucket,
                relevance,
                age_days: (now - m.created_at).num_days(),
            },
            subject: topic_key.as_ref().and_then(|t| {
                subjects
                    .get(&(m.scope, m.scope_key.clone(), t.clone()))
                    .map(|read| subject_info(read, m.id))
            }),
            topic_key,
            value_key: r.try_get("value_key").unwrap_or(None),
            temporal: q.as_of.map(|_| temporal_of(r, m.state)),
            importance: rows::enum_val(r, "importance").unwrap_or(Importance::Normal),
            pinned: rows::boolean(r, "pinned").unwrap_or(false),
            verification: crate::evidence::local_view(
                store,
                m.id,
                rows::enum_val(r, "verification").unwrap_or(VerificationState::Unverified),
                r.try_get::<Option<String>, _>("verification_authority")
                    .unwrap_or(None)
                    .and_then(|a| a.parse::<VerificationAuthority>().ok()),
                r.try_get("last_verified_at").unwrap_or(None),
            )
            .await?,
            reinforcement: cairn_core::wire::Reinforcement {
                count: r.try_get("reinforcement_count").unwrap_or(0),
                distinct_origins: r.try_get("distinct_origin_count").unwrap_or(0),
            },
        });
    }
    Ok(out)
}

/// Where one result stands among the other answers to its subject.
///
/// The split is the one the subject state itself rests on: an answer asserting
/// a **different** value competes, and one asserting the **same** value in
/// different words corroborates. Cairn merges neither (D46), so a caller that
/// wants a single answer has to be told what else is there and which kind it
/// is — otherwise it picks one and calls it the project's position, which is
/// the silent winner this feature exists to prevent.
fn subject_info(read: &crate::knowledge::SubjectRead, id: Uuid) -> cairn_core::wire::SubjectInfo {
    let value_of = |m: Uuid| -> Option<String> {
        read.members
            .iter()
            .find(|x| x.id == m)
            .and_then(|x| x.value_key.clone())
    };
    let mine = value_of(id);
    let mut competing = Vec::new();
    let mut corroborating = Vec::new();
    for answer in &read.view.answers {
        if *answer == id {
            continue;
        }
        // Two members with no value key at all agree about nothing in
        // particular, so absence never counts as agreement.
        if mine.is_some() && value_of(*answer) == mine {
            corroborating.push(*answer);
        } else {
            competing.push(*answer);
        }
    }
    cairn_core::wire::SubjectInfo {
        reconciliation: read.view.reconciliation,
        is_canonical_answer: read.view.answers.contains(&id),
        competing_answers: competing,
        corroborating_answers: corroborating,
    }
}

/// How many rows a derived-subject filter may consider.
///
/// A subject state cannot be a SQL predicate, so the filter reads candidates
/// and derives. The bound is what keeps that from becoming an unbounded scan
/// (FR-474's discipline applied to a read).
const SUBJECT_FILTER_SCAN_MAX: i64 = 512;

/// The subjects among the candidate rows whose derived state qualifies.
///
/// Returns the derivation, not just the key: every qualifying subject has just
/// been derived, and every surviving row needs that same derivation for its
/// `subject` field. Discarding it here only to recompute it twenty lines later
/// would double the work the bound above exists to limit.
async fn qualifying_subjects(
    store: &Store,
    project_id: Uuid,
    raw: &[sqlx::sqlite::SqliteRow],
    conflicted: bool,
    corroborated: bool,
) -> Result<std::collections::BTreeMap<(MemoryScope, String, String), crate::knowledge::SubjectRead>>
{
    let mut subjects: std::collections::BTreeSet<(MemoryScope, String, String)> =
        Default::default();
    for r in raw {
        let scope: MemoryScope = rows::enum_val(r, "scope")?;
        let scope_key: String = r.try_get("scope_key")?;
        if let Some(topic) = r.try_get::<Option<String>, _>("topic_key")? {
            subjects.insert((scope, scope_key, topic));
        }
    }

    let mut keep = std::collections::BTreeMap::new();
    for (scope, scope_key, topic) in subjects {
        let read = crate::knowledge::subject(
            store,
            project_id,
            scope,
            &scope_key,
            &topic,
            crate::repo::DEFAULT_RECONCILE_MEMBERS_MAX,
        )
        .await?;
        let qualifies = (conflicted
            && read.view.reconciliation == cairn_core::Reconciliation::Conflicted)
            || (corroborated
                && read.view.reconciliation == cairn_core::Reconciliation::Corroborated);
        if qualifies {
            keep.insert((scope, scope_key, topic), read);
        }
    }
    Ok(keep)
}

/// What a historical answer may say about when this proposal applied.
///
/// `stale_at` NULL means **unknown**, never "not stale": a memory that went
/// stale before Cairn recorded staleness instants has no authoritative
/// instant, so the answer says the applicability is unknown rather than
/// implying the proposal applied throughout (FR-342, D82).
fn temporal_of(r: &sqlx::sqlite::SqliteRow, state: MemoryState) -> cairn_core::wire::Temporal {
    let parse = |col: &str| -> Option<DateTime<Utc>> {
        r.try_get::<Option<String>, _>(col)
            .ok()
            .flatten()
            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
            .map(|d| d.with_timezone(&Utc))
    };
    let stale_at = parse("stale_at");
    let applicability = if state == MemoryState::Stale && stale_at.is_none() {
        cairn_core::Applicability::Unknown
    } else {
        cairn_core::Applicability::Bounded
    };
    cairn_core::wire::Temporal {
        effective_from: parse("effective_from"),
        superseded_at: parse("superseded_at"),
        stale_at,
        applicability,
    }
}

/// Turn user text into an FTS5 query.
///
/// Terms are quoted so punctuation cannot be read as FTS5 syntax, and joined
/// with OR so a multi-word query still recalls partial matches.
fn fts_query(input: &str) -> String {
    let terms: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if terms.is_empty() {
        // Matches nothing rather than erroring on empty FTS syntax.
        return "\"\"".to_string();
    }
    terms.join(" OR ")
}

/// Memory content for one scope, most relevant first — what the briefing uses.
pub async fn memory_for_scope(
    store: &Store,
    project_id: Uuid,
    scope: MemoryScope,
    scope_key: &str,
    limit: i64,
) -> Result<Vec<Memory>> {
    let rs = sqlx::query(
        "SELECT * FROM memories
         WHERE project_id = ?1 AND scope = ?2 AND scope_key = ?3
           AND state = 'active' AND deleted_at IS NULL
         ORDER BY created_at DESC LIMIT ?4",
    )
    .bind(project_id.to_string())
    .bind(scope.as_str())
    .bind(scope_key)
    .bind(limit)
    .fetch_all(store.pool())
    .await?;
    rs.iter().map(rows::memory_bare).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbox::SyncPolicy;
    use crate::repo::*;

    /// Tests run against an unlinked project: nothing may leave the machine.
    const LOCAL: SyncPolicy = SyncPolicy {
        linked: false,
        server_project_id: None,
    };

    async fn fixture() -> (Store, Uuid, Uuid, Uuid) {
        let store = Store::open_memory().await.unwrap();
        let user = ensure_local_user(&store).await.unwrap();
        let p = ensure_project(&store, "/tmp/x/.git", "x", None)
            .await
            .unwrap();
        let t = create_task(
            &store,
            p.id,
            "Rate limit",
            "429 over limit",
            &[],
            new_id(),
            LOCAL,
        )
        .await
        .unwrap();
        let s = start_session(
            &store,
            StartSession {
                project_id: p.id,
                user_id: user,
                agent: "claude-code",
                agent_session_key: "k1",
                branch: "main",
                commit_sha: None,
                worktree_path: "/tmp/x",
                task_id: Some(t.id),
                daemon_run_id: new_id(),
                policy: LOCAL,
            },
        )
        .await
        .unwrap();
        (store, p.id, t.id, s.id)
    }

    async fn add(
        store: &Store,
        project: Uuid,
        session: Uuid,
        scope: MemoryScope,
        key: &str,
        content: &str,
    ) -> Memory {
        create_memory(
            store,
            NewMemory {
                project_id: project,
                kind: MemoryType::Fact,
                scope,
                scope_key: key,
                content,
                origin_session_id: session,
                local_only: false,
                evidence: &[],
                topic_key: None,
                value_key: None,
                importance: cairn_core::Importance::Normal,
            },
            LOCAL,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn scope_precedence_beats_relevance() {
        let (store, project, task, session) = fixture().await;
        // The project-scoped memory is the better lexical match on purpose.
        add(
            &store,
            project,
            session,
            MemoryScope::Project,
            &project.to_string(),
            "tests tests tests always fail without the queue",
        )
        .await;
        add(
            &store,
            project,
            session,
            MemoryScope::Branch,
            "main",
            "tests need a branch fixture",
        )
        .await;
        add(
            &store,
            project,
            session,
            MemoryScope::Task,
            &task.to_string(),
            "tests here",
        )
        .await;

        let ctx = SearchContext {
            branch: Some("main".into()),
            task_id: Some(task),
            session_id: Some(session),
        };
        let q = MemoryQuery {
            query: Some("tests".into()),
            ..Default::default()
        };
        let out = search(&store, project, &q, &ctx).await.unwrap();

        assert_eq!(out.len(), 3);
        assert_eq!(out[0].scope, MemoryScope::Task, "task scope must lead");
        assert_eq!(out[1].scope, MemoryScope::Branch);
        assert_eq!(out[2].scope, MemoryScope::Project);
    }

    #[tokio::test]
    async fn only_active_memories_are_returned_by_default() {
        let (store, project, _task, session) = fixture().await;
        let m = add(
            &store,
            project,
            session,
            MemoryScope::Project,
            &project.to_string(),
            "outbox chosen",
        )
        .await;
        supersede_memory(
            &store,
            m.id,
            NewMemory {
                project_id: project,
                kind: MemoryType::Decision,
                scope: MemoryScope::Project,
                scope_key: &project.to_string(),
                content: "dual writes rejected, outbox chosen",
                origin_session_id: session,
                local_only: false,
                evidence: &[],
                topic_key: None,
                value_key: None,
                importance: cairn_core::Importance::Normal,
            },
            LOCAL,
        )
        .await
        .unwrap();

        let q = MemoryQuery {
            query: Some("outbox".into()),
            ..Default::default()
        };
        let out = search(&store, project, &q, &SearchContext::default())
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].state, MemoryState::Active);

        let q2 = MemoryQuery {
            query: Some("outbox".into()),
            state: Some(MemoryState::Superseded),
            ..Default::default()
        };
        let sup = search(&store, project, &q2, &SearchContext::default())
            .await
            .unwrap();
        assert_eq!(sup.len(), 1);
        assert!(sup[0].superseded_by_id.is_some(), "original keeps the link");
    }

    #[tokio::test]
    async fn provenance_is_present_with_zero_evidence() {
        // Manual MCP mode records memory with no observations (FR-019).
        let (store, project, _task, session) = fixture().await;
        add(
            &store,
            project,
            session,
            MemoryScope::Project,
            &project.to_string(),
            "a convention",
        )
        .await;
        let q = MemoryQuery {
            query: Some("convention".into()),
            ..Default::default()
        };
        let out = search(&store, project, &q, &SearchContext::default())
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].provenance.evidence_count, 0);
        assert_eq!(out[0].provenance.session_id, session);
    }

    #[tokio::test]
    async fn filters_apply_without_a_text_query() {
        let (store, project, _task, session) = fixture().await;
        add(
            &store,
            project,
            session,
            MemoryScope::Project,
            &project.to_string(),
            "one",
        )
        .await;
        create_memory(
            &store,
            NewMemory {
                project_id: project,
                kind: MemoryType::Failure,
                scope: MemoryScope::Project,
                scope_key: &project.to_string(),
                content: "two",
                origin_session_id: session,
                local_only: false,
                evidence: &[],
                topic_key: None,
                value_key: None,
                importance: cairn_core::Importance::Normal,
            },
            LOCAL,
        )
        .await
        .unwrap();

        let q = MemoryQuery {
            kind: Some(MemoryType::Failure),
            ..Default::default()
        };
        let out = search(&store, project, &q, &SearchContext::default())
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "two");
    }

    #[tokio::test]
    async fn punctuation_in_a_query_does_not_break_fts() {
        let (store, project, _task, session) = fixture().await;
        add(
            &store,
            project,
            session,
            MemoryScope::Project,
            &project.to_string(),
            "run cargo test --workspace before pushing",
        )
        .await;
        let q = MemoryQuery {
            query: Some("cargo test --workspace".into()),
            ..Default::default()
        };
        let out = search(&store, project, &q, &SearchContext::default())
            .await
            .unwrap();
        assert_eq!(out.len(), 1);
    }
}
