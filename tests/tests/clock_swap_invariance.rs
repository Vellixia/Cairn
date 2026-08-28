//! T097 — the merge corpus, run against two real stores whose clocks disagree
//! (SC-304, SC-324).
//!
//! Every scenario in `tests/knowledge/merge/` ships with a **clock-reversed
//! twin**: the same offline writes, with the two machines' clocks swapped. The
//! pair is the assertion. If any merge step consulted a timestamp to choose a
//! winner, the twin would produce a different answer, so a byte-identical
//! merged state across the pair is what proves no clock decides anything
//! (FR-303, FR-411).
//!
//! The exchange between the two stores is driven by the **outbox**, not by
//! copying rows. That is what makes `a_local_only_memory_never_travels` an
//! honest test rather than an assertion about the test's own filter: a
//! `local_only` memory produces no outbox row at all, and the test would have
//! to invent one to move it.

use cairn_core::domain::{
    Importance, MemoryScope, MemoryType, OutboxEntityType, RelationBasis, RelationKind,
};
use cairn_store::outbox::SyncPolicy;
use cairn_store::{knowledge, outbox, repo, Store};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("knowledge/merge")
}

/// Every case in a directory, in filename order.
fn cases_in(dir: &Path) -> Vec<(String, Value)> {
    let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|p| {
            let name = p
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned();
            let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{name}: {e}"));
            let value: Value =
                serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name} is not JSON: {e}"));
            (name, value)
        })
        .collect()
}

/// A case and its clock-reversed twin, paired by filename.
///
/// A case without a twin is a corpus defect, not something to skip: the pair
/// **is** the invariance assertion, so a lone case would silently assert
/// nothing.
fn pairs_in(dir: &Path) -> Vec<((String, Value), (String, Value))> {
    let cases = cases_in(dir);
    assert!(
        !cases.is_empty(),
        "no cases in {} — the invariance test would pass vacuously",
        dir.display()
    );
    assert_eq!(
        cases.len() % 2,
        0,
        "{} holds an odd number of cases, so one has no clock-reversed twin",
        dir.display()
    );
    cases
        .chunks(2)
        .map(|pair| {
            let (a, b) = (pair[0].clone(), pair[1].clone());
            assert!(
                b.0.contains("clock_reversed"),
                "{} is not the clock-reversed twin of {}",
                b.0,
                a.0
            );
            assert!(
                !a.0.contains("clock_reversed"),
                "{} is a twin with no original",
                a.0
            );
            (a, b)
        })
        .collect()
}

fn extra<'v>(case: &'v Value, side: &str) -> &'v Value {
    case.get(side)
        .and_then(|s| s.get("extra"))
        .unwrap_or(&Value::Null)
}

// ---------------------------------------------------------------------------
// Running one scenario against two real stores
// ---------------------------------------------------------------------------

struct Machine {
    store: Store,
    _dir: tempfile::TempDir,
    clock: String,
    session: Uuid,
}

impl Machine {
    async fn open(project: Uuid, clock: &str) -> Self {
        let dir = tempfile::tempdir().expect("dir");
        let store = Store::open(&dir.path().join("cairn.sqlite3"))
            .await
            .expect("store");
        let now = chrono::Utc::now().to_rfc3339();
        // Linked, so the writes below produce real outbox rows. The exchange
        // reads those rows, which is what puts the privacy boundary in the
        // path of this test rather than beside it.
        sqlx::query(
            "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                                   server_project_id, created_at, updated_at, deleted_at)
             VALUES (?1, 'merge-fixture', ?2, NULL, 1, ?3, ?4, ?4, NULL)",
        )
        .bind(project.to_string())
        .bind(format!("/fixture/{project}/.git"))
        .bind(project.to_string())
        .bind(&now)
        .execute(store.pool())
        .await
        .expect("project");

        Self {
            store,
            _dir: dir,
            clock: clock.to_string(),
            session: Uuid::now_v7(),
        }
    }

    fn policy(&self, project: Uuid) -> SyncPolicy {
        SyncPolicy {
            linked: true,
            server_project_id: Some(project),
        }
    }

    /// Write a proposal, then stamp it with this machine's clock.
    ///
    /// The stamp is the whole point of the fixture pair: two machines whose
    /// clocks disagree by months must still merge to the same answer.
    async fn propose(&self, project: Uuid, spec: &Value) -> Uuid {
        let topic = spec.get("topic").and_then(|v| v.as_str());
        let value_key = spec.get("value").and_then(|v| v.as_str());
        let content = spec
            .get("content")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| match (topic, value_key) {
                (Some(t), Some(v)) => format!("{t} is {v}"),
                _ => "an undifferentiated claim".to_string(),
            });
        let local_only = spec
            .get("local_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let scope_key = project.to_string();

        let memory = repo::create_memory(
            &self.store,
            repo::NewMemory {
                project_id: project,
                kind: MemoryType::Decision,
                scope: MemoryScope::Project,
                scope_key: &scope_key,
                content: &content,
                origin_session_id: self.session,
                local_only,
                evidence: &[],
                topic_key: topic,
                value_key,
                importance: Importance::Normal,
            },
            self.policy(project),
        )
        .await
        .expect("proposal");

        sqlx::query(
            "UPDATE memories SET created_at = ?2, updated_at = ?2, effective_from = ?2
              WHERE id = ?1",
        )
        .bind(memory.id.to_string())
        .bind(&self.clock)
        .execute(self.store.pool())
        .await
        .expect("clock");

        memory.id
    }

    async fn record(&self, project: Uuid, kind: RelationKind, from: Uuid, to: Uuid) {
        knowledge::record_relation(
            &self.store,
            knowledge::NewRelation {
                project_id: project,
                from,
                to,
                kind,
                decided_by_session: self.session,
                basis: RelationBasis::ExplicitAgent,
                basis_evidence_id: None,
                rationale: None,
            },
        )
        .await
        .expect("relation");
    }
}

/// Deliver everything one machine has queued to the other.
///
/// Relations whose endpoints have not arrived are **held**, exactly as the
/// daemon holds them, and the caller drains again after the memories land.
async fn deliver(from: &Machine, to: &Machine, project: Uuid) {
    let items = outbox::claim(&from.store, project, 500)
        .await
        .expect("claim");
    let mut endpoints: Vec<Uuid> = Vec::new();

    for (id, item) in &items {
        match item.entity_type {
            OutboxEntityType::Memory => {
                let p = &item.payload;
                let s = |k: &str| p.get(k).and_then(|v| v.as_str());
                let scope_key = s("scope_key").unwrap_or_default().to_string();
                repo::import_memory(
                    &to.store,
                    repo::ImportedMemory {
                        id: item.entity_id,
                        project_id: project,
                        kind: s("type")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(MemoryType::Fact),
                        scope: s("scope")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(MemoryScope::Project),
                        scope_key: &scope_key,
                        content: s("content").unwrap_or_default(),
                        origin_session_id: p
                            .get("provenance")
                            .and_then(|v| v.get("session_id"))
                            .and_then(|v| v.as_str())
                            .and_then(|v| Uuid::parse_str(v).ok())
                            .unwrap_or_else(Uuid::now_v7),
                        topic_key: s("topic_key"),
                        value_key: s("value_key"),
                        importance: s("importance")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(Importance::Normal),
                        effective_from: s("effective_from"),
                    },
                )
                .await
                .expect("import memory");
            }
            OutboxEntityType::MemoryRelation => {
                let p = &item.payload;
                let uuid = |k: &str| {
                    p.get(k)
                        .and_then(|v| v.as_str())
                        .and_then(|v| Uuid::parse_str(v).ok())
                };
                let (Some(a), Some(b)) = (uuid("from_memory_id"), uuid("to_memory_id")) else {
                    continue;
                };
                // Held, not dropped: the next drain carries it again.
                if repo::memory(&to.store, a).await.is_err()
                    || repo::memory(&to.store, b).await.is_err()
                {
                    continue;
                }
                let kind: RelationKind = p
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .and_then(|v| v.parse().ok())
                    .expect("a relation kind the domain defines");
                to.record(project, kind, a, b).await;
                endpoints.extend([a, b]);
            }
            _ => {}
        }
        outbox::mark_delivered(&from.store, *id)
            .await
            .expect("delivered");
    }

    // The arriving decisions changed what is canonical, so the derived columns
    // are rebuilt from the records rather than patched.
    let _ = knowledge::rebuild_supersession(&to.store, project).await;
    for id in endpoints {
        let _ = knowledge::rebuild_reinforcement(&to.store, id).await;
    }
}

/// The merged state of one machine, rendered so two machines' answers can be
/// compared byte for byte.
///
/// Nothing time-derived appears in it. A field that moved with the clock would
/// make every pair differ and the invariance assertion meaningless — and a
/// field that *should* move with the clock has no business deciding what is
/// canonical.
async fn rendered(m: &Machine, project: Uuid) -> String {
    let mut subjects = knowledge::subject_keys(&m.store, project)
        .await
        .expect("subject keys");
    subjects.sort_by(|a, b| (a.0.as_str(), &a.1, &a.2).cmp(&(b.0.as_str(), &b.1, &b.2)));

    let mut out = String::new();
    let mut seen: BTreeMap<(String, String, String), ()> = BTreeMap::new();
    for (scope, scope_key, topic) in subjects {
        let key = (scope.as_str().to_string(), scope_key.clone(), topic.clone());
        if seen.insert(key, ()).is_some() {
            continue;
        }
        let read = knowledge::subject(&m.store, project, scope, &scope_key, &topic, 64)
            .await
            .expect("subject");

        out.push_str(&format!(
            "{topic} {:?} answers={} narrowed={}\n",
            read.view.reconciliation,
            read.view.answers.len(),
            read.view.narrowed_by.len()
        ));
        // Members by content, not by id: two machines minted different ids for
        // the records each of them authored, and comparing ids would make the
        // pair differ for a reason that has nothing to do with clocks.
        let mut members: Vec<String> = read
            .members
            .iter()
            .map(|f| {
                format!(
                    "  member value={:?} state={:?} verification={:?}",
                    f.value_key, f.state, f.verification
                )
            })
            .collect();
        members.sort();
        out.push_str(&members.join("\n"));
        out.push('\n');

        // The decision set: what was decided, between which members.
        //
        // `basis` is deliberately **not** part of it. It records how *this*
        // machine came to hold the decision, and two machines legitimately
        // arrive at one decision by different routes — Cairn detects a conflict
        // itself while an agent on the other machine asserts the same one. The
        // primary key is `(from, to, kind)`, so the two collapse to one durable
        // row on each machine with whichever basis that machine wrote first.
        // Requiring those to match would be requiring the machines to have had
        // the same history, which is the opposite of what convergence means.
        let value_of = |id: &Uuid| {
            read.members
                .iter()
                .find(|m| &m.id == id)
                .and_then(|m| m.value_key.clone())
                .unwrap_or_else(|| "?".to_string())
        };
        let mut decisions: Vec<String> = read
            .view
            .decisions
            .iter()
            .map(|r| {
                format!(
                    "  decision {:?} {} {}",
                    r.kind,
                    value_of(&r.from),
                    value_of(&r.to)
                )
            })
            .collect();
        decisions.sort();
        out.push_str(&decisions.join("\n"));
        out.push('\n');
    }
    out
}

async fn memory_count(m: &Machine, project: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM memories WHERE project_id = ?1 AND deleted_at IS NULL",
    )
    .bind(project.to_string())
    .fetch_one(m.store.pool())
    .await
    .expect("count")
}

async fn relation_count(m: &Machine, project: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM memory_relations WHERE project_id = ?1 AND deleted_at IS NULL",
    )
    .bind(project.to_string())
    .fetch_one(m.store.pool())
    .await
    .expect("count")
}

async fn superseded_count(m: &Machine, project: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM memories WHERE project_id = ?1 AND state = 'superseded'",
    )
    .bind(project.to_string())
    .fetch_one(m.store.pool())
    .await
    .expect("count")
}

/// What one run of a scenario produced.
struct Outcome {
    rendered_a: String,
    rendered_b: String,
    memories_a: i64,
    memories_b: i64,
    relations_a: i64,
    superseded_a: i64,
}

async fn run(case: &Value) -> Outcome {
    let input = extra(case, "input");
    let clocks = input
        .get("clocks")
        .and_then(|v| v.as_array())
        .expect("a clock pair");
    let project = Uuid::now_v7();
    let a = Machine::open(project, clocks[0].as_str().expect("clock a")).await;
    let b = Machine::open(project, clocks[1].as_str().expect("clock b")).await;

    // --- Offline. Each machine writes what it knows, at its own clock.
    let empty = Vec::new();
    let proposals_of = |side: &str| -> Vec<Value> {
        input
            .get(side)
            .and_then(|s| s.get("proposals"))
            .and_then(|v| v.as_array())
            .unwrap_or(&empty)
            .clone()
    };

    let mut global: Vec<Uuid> = Vec::new();
    for spec in proposals_of("store_a") {
        global.push(a.propose(project, &spec).await);
    }
    let a_count = global.len();
    for spec in proposals_of("store_b") {
        global.push(b.propose(project, &spec).await);
    }

    // --- The proposals travel, so both machines hold the same member set. A
    //     relation recorded next names records both of them have.
    deliver(&a, &b, project).await;
    deliver(&b, &a, project).await;

    // --- Each machine records its own decisions, offline, against the shared
    //     member set. Indices address the global proposal order: machine A's
    //     proposals, then machine B's.
    let relations_of = |side: &str| -> Vec<Value> {
        input
            .get(side)
            .and_then(|s| s.get("relations"))
            .and_then(|v| v.as_array())
            .unwrap_or(&empty)
            .clone()
    };
    for (machine, side) in [(&a, "store_a"), (&b, "store_b")] {
        for spec in relations_of(side) {
            let idx = |k: &str| spec.get(k).and_then(|v| v.as_u64()).expect("an index") as usize;
            let kind: RelationKind = spec
                .get("kind")
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse().ok())
                .expect("a relation kind the domain defines");
            let (from, to) = (global[idx("from")], global[idx("to")]);
            // A machine can only decide about records it holds. A `local_only`
            // proposal never reached the other machine, and a fixture naming
            // one across the boundary would be describing something that
            // cannot happen.
            if repo::memory(&machine.store, from).await.is_err()
                || repo::memory(&machine.store, to).await.is_err()
            {
                continue;
            }
            machine.record(project, kind, from, to).await;
        }
    }
    let _ = knowledge::rebuild_supersession(&a.store, project).await;
    let _ = knowledge::rebuild_supersession(&b.store, project).await;

    // --- And the decisions travel.
    deliver(&a, &b, project).await;
    deliver(&b, &a, project).await;

    let _ = a_count;
    Outcome {
        rendered_a: rendered(&a, project).await,
        rendered_b: rendered(&b, project).await,
        memories_a: memory_count(&a, project).await,
        memories_b: memory_count(&b, project).await,
        relations_a: relation_count(&a, project).await,
        superseded_a: superseded_count(&a, project).await,
    }
}

/// Assert a case's own stated expectations.
fn check_expectations(name: &str, case: &Value, o: &Outcome) {
    let expect = extra(case, "expect");
    let n = |k: &str| expect.get(k).and_then(|v| v.as_i64());

    if let Some(want) = n("memories") {
        assert_eq!(
            o.memories_a, want,
            "{name}: machine A holds {} memories, expected {want}",
            o.memories_a
        );
        assert_eq!(
            o.memories_b, want,
            "{name}: machine B holds {} memories, expected {want}",
            o.memories_b
        );
    }
    if let Some(want) = n("memories_on_b") {
        assert_eq!(
            o.memories_b, want,
            "{name}: machine B holds {} memories, expected {want}",
            o.memories_b
        );
    }
    if let Some(want) = n("relations") {
        assert_eq!(
            o.relations_a, want,
            "{name}: {} relations survived, expected {want}",
            o.relations_a
        );
    }
    if let Some(want) = n("superseded") {
        assert_eq!(
            o.superseded_a, want,
            "{name}: {} memories superseded, expected {want}",
            o.superseded_a
        );
    }
    if let Some(want) = expect.get("reconciliation").and_then(|v| v.as_str()) {
        // Rendered as `Debug`, which is the enum variant name.
        let wanted = want.replace('_', "");
        for (who, text) in [("A", &o.rendered_a), ("B", &o.rendered_b)] {
            assert!(
                text.to_lowercase().contains(&wanted.to_lowercase()),
                "{name}: machine {who} did not report `{want}`:\n{text}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// Every canonical-knowledge scenario merges identically under reversed clocks.
#[test]
fn the_merge_corpus_is_clock_independent() {
    let rt = runtime();
    rt.block_on(async {
        for ((name, case), (twin_name, twin)) in pairs_in(&corpus_root()) {
            let first = run(&case).await;
            let second = run(&twin).await;

            assert_eq!(
                first.rendered_a, second.rendered_a,
                "{name} and {twin_name} disagree on machine A's merged state, so \
                 something in the merge consulted a clock"
            );
            assert_eq!(
                first.rendered_b, second.rendered_b,
                "{name} and {twin_name} disagree on machine B's merged state, so \
                 something in the merge consulted a clock"
            );
            assert_eq!(
                (first.relations_a, first.superseded_a),
                (second.relations_a, second.superseded_a),
                "{name} and {twin_name} produced different decision sets"
            );

            check_expectations(&name, &case, &first);
            check_expectations(&twin_name, &twin, &second);
        }
    });
}

/// Both machines converge to the **same** merged state, not merely to a
/// clock-independent one.
///
/// A merge that lost the same record on both machines would satisfy the twin
/// assertion above and still be wrong.
#[test]
fn both_machines_agree_after_the_merge() {
    let rt = runtime();
    rt.block_on(async {
        for (name, case) in cases_in(&corpus_root()) {
            // The one case where the two machines are *meant* to differ: a
            // local-only memory stays where it was written.
            if name.contains("local_only") {
                continue;
            }
            let o = run(&case).await;
            assert_eq!(
                o.rendered_a, o.rendered_b,
                "{name}: the two machines did not converge"
            );
        }
    });
}

/// One conflict detected independently on both machines converges to exactly
/// **one** durable relation (D78, SC-304).
#[test]
fn symmetric_relation() {
    let rt = runtime();
    rt.block_on(async {
        let dir = corpus_root().join("symmetric_relation");
        for ((name, case), (twin_name, twin)) in pairs_in(&dir) {
            let first = run(&case).await;
            let second = run(&twin).await;

            assert_eq!(
                first.relations_a, 1,
                "{name}: two mirror-image rows survived instead of one"
            );
            assert_eq!(
                second.relations_a, 1,
                "{twin_name}: two mirror-image rows survived instead of one"
            );
            assert_eq!(
                first.rendered_a, second.rendered_a,
                "{name} and {twin_name} disagree, so the endpoint normalization \
                 depends on which machine's clock ran ahead"
            );
            assert_eq!(
                first.rendered_a, first.rendered_b,
                "{name}: the two machines did not converge on one decision"
            );

            check_expectations(&name, &case, &first);
            check_expectations(&twin_name, &twin, &second);
        }
    });
}

// ===========================================================================
// T139 — the same invariance, extended to the two new domains (FR-492, FR-493)
// ===========================================================================
//
// The corpus above covers project memory. Personal and team knowledge reuse the
// same `classify_proposal`/`derive_subject` machinery, so the invariant ought to
// carry over — but "ought to" is the word this test exists to remove. The two new
// domains reach that machinery through different write paths, and the team domain
// has something project memory does not: a lifecycle with `ratified_at` and
// `retired_at` timestamps sitting right next to the records being ordered. A
// comparator that reached for the nearest available timestamp would find one
// here and nowhere else.

/// Swapping the clocks on two personal records changes nothing about the answer
/// derived from them.
///
/// Falsified by any ordering on `created_at` in the personal read path.
#[test]
fn personal_knowledge_is_clock_independent() {
    runtime().block_on(async {
        use cairn_store::global::{
            merge_synced_personal, personal_subject, SyncedPersonalKnowledge,
        };

        let owner = cairn_core::domain::new_id();
        let early = chrono::Utc::now() - chrono::Duration::hours(6);
        let late = chrono::Utc::now();

        // The same two records, merged twice with the timestamps exchanged.
        let mut rendered: Vec<String> = Vec::new();
        for (first_at, second_at) in [(early, late), (late, early)] {
            let store = Store::open_memory().await.unwrap();
            for (content, value, at) in [
                ("the retry budget is four attempts", "four", first_at),
                ("the retry budget is two attempts", "two", second_at),
            ] {
                merge_synced_personal(
                    &store,
                    SyncedPersonalKnowledge {
                        id: cairn_core::domain::new_id(),
                        owner_user_id: owner,
                        knowledge_type: MemoryType::Fact,
                        content: content.into(),
                        topic_key: Some("retry.budget".into()),
                        value_key: Some(value.into()),
                        applicability: Vec::new(),
                        writer_id: cairn_core::domain::new_id(),
                        writer_seq: 1,
                        created_at: at,
                        superseded_by_id: None,
                        forgotten_at: None,
                    },
                )
                .await
                .expect("merge");
            }

            let subject = personal_subject(&store, owner, "retry.budget")
                .await
                .expect("subject");
            rendered.push(format!(
                "{:?}|{}",
                subject.view.reconciliation,
                subject.members.len()
            ));
        }
        assert_eq!(
            rendered[0], rendered[1],
            "swapping two personal records' clocks changed the derived answer"
        );
    });
}

/// Ratification order does not decide which of two disagreeing team entries is
/// the answer, and neither does the clock.
///
/// The two are asserted together because they are the same mistake in two forms:
/// "whichever was ratified more recently" is a write-order rule, and
/// "whichever has the later `ratified_at`" is a timestamp rule, and this domain is
/// the only one in Cairn that offers both.
///
/// Falsified by any code path that orders two `authoritative` rows to pick a
/// winner.
#[test]
fn team_ratification_order_and_clock_decide_nothing() {
    runtime().block_on(async {
        use cairn_store::global::{propose_team, ratify_team, team_subject, NewTeamKnowledge};

        let mut rendered: Vec<String> = Vec::new();
        for ratify_the_second_first in [false, true] {
            let store = Store::open_memory().await.unwrap();
            let admin = cairn_core::domain::new_id();

            let mut ids = Vec::new();
            for (content, value) in [
                (
                    "commit messages follow Conventional Commits",
                    "conventional",
                ),
                ("commit messages are free-form", "free_form"),
            ] {
                let outcome = propose_team(
                    &store,
                    NewTeamKnowledge::direct(
                        cairn_core::domain::new_id(),
                        MemoryType::Convention,
                        content,
                        Some("style.commit_message"),
                        Some(value),
                        Vec::new(),
                    ),
                    &[],
                )
                .await
                .expect("propose");
                ids.push(outcome.record.id);
            }
            if ratify_the_second_first {
                ids.reverse();
            }
            for id in &ids {
                ratify_team(&store, *id, admin, None).await.expect("ratify");
            }

            // Then move the recorded ratification clocks so the *other* one
            // looks newer, which a timestamp rule would notice and a correct
            // implementation cannot.
            let earlier = (chrono::Utc::now() - chrono::Duration::hours(6)).to_rfc3339();
            sqlx::query("UPDATE team_knowledge SET ratified_at = ?1 WHERE id = ?2")
                .bind(&earlier)
                .bind(ids[1].to_string())
                .execute(store.pool())
                .await
                .unwrap();

            let subject = team_subject(&store, "style.commit_message")
                .await
                .expect("subject");
            let mut answers = subject.view.answers.clone();
            answers.sort();
            rendered.push(format!(
                "{:?}|{}|{}",
                subject.view.reconciliation,
                subject.members.len(),
                answers.len()
            ));
        }
        assert_eq!(
            rendered[0], rendered[1],
            "ratification order changed which team entries are the answer"
        );
        assert!(
            rendered[0].contains("Conflicted"),
            "two disagreeing authoritative entries did not surface as a conflict: {}",
            rendered[0]
        );
    });
}
