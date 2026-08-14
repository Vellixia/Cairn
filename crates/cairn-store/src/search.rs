//! Memory retrieval: exact filters, FTS5/BM25 relevance, scope-first ranking
//! (FR-022 – FR-026, D3).
//!
//! Scope dominates. A mediocre match about *this task* beats an excellent match
//! about an unrelated one — that is what makes Cairn's recall correct rather
//! than merely similar.

use crate::{repo, rows, Result, Store};
use cairn_core::domain::*;
use cairn_core::wire::{MemoryQuery, MemoryResult, Provenance, RankInfo};
use chrono::Utc;
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
    sql.push_str(" AND m.project_id = ? AND m.deleted_at IS NULL AND m.state = ?");

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

    if q.kind.is_some() {
        sql.push_str(" AND m.type = ?");
    }
    sql.push_str(" ORDER BY scope_bucket ASC, relevance DESC, m.created_at DESC LIMIT ?");

    let mut query = sqlx::query(&sql);
    if let Some(text) = &q.query {
        query = query.bind(fts_query(text));
    }
    query = query.bind(project_id.to_string()).bind(state.as_str());

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
    if let Some(kind) = q.kind {
        query = query.bind(kind.as_str());
    }
    query = query.bind(limit);

    let raw = query.fetch_all(store.pool()).await?;
    let now = Utc::now();
    let mut out = Vec::with_capacity(raw.len());
    for r in &raw {
        let m = rows::memory_bare(r)?;
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
        });
    }
    Ok(out)
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
        let t = create_task(&store, p.id, "Rate limit", "429 over limit", &[], LOCAL)
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
