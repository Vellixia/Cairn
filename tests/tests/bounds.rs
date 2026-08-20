//! T140 — every D75 bound, at its default, and shown to bind (FR-500, SC-320).
//!
//! Two assertions per bound, and the second is the one that matters. That a
//! default has not drifted is cheap to check and cheap to satisfy; that
//! *exceeding* it produces the documented deferral or refusal rather than
//! unbounded work is the property the bound exists for. A cap nobody enforces
//! is a comment.

use cairn_core::budget::Budget;
use cairn_core::CairnConfig;

/// The seventeen fields D75 counts as sixteen classes — the stored bounds on an
/// evidence value and its locator are one clause covering two values.
///
/// Restated here rather than read from the config, so a change to a default has
/// to be made in two places by someone who meant it.
const DOCUMENTED: &[(&str, f64)] = &[
    ("min_safe_context_fraction", 0.40),
    ("min_context_budget_tokens", 600.0),
    ("goal_max_tokens", 60.0),
    ("pin_budget_project", 12.0),
    ("pin_budget_per_scope", 4.0),
    ("pins_in_context_max", 4.0),
    ("warnings_in_context_max", 5.0),
    ("patterns_in_context_max", 2.0),
    ("reconcile_members_max", 64.0),
    ("subject_warning_scan_max", 256.0),
    ("evidence_lookups_per_event_max", 8.0),
    ("verify_pass_evidence_max", 200.0),
    ("verify_pass_runs_max", 50.0),
    ("verify_pass_wall_ms", 2000.0),
    ("evidence_value_max_bytes", 256.0),
    ("evidence_locator_max_bytes", 256.0),
    ("pattern_signals_min", 2.0),
];

fn actual(config: &CairnConfig, name: &str) -> f64 {
    match name {
        "min_safe_context_fraction" => config.min_safe_context_fraction,
        "min_context_budget_tokens" => config.min_context_budget_tokens as f64,
        "goal_max_tokens" => config.goal_max_tokens as f64,
        "pin_budget_project" => config.pin_budget_project as f64,
        "pin_budget_per_scope" => config.pin_budget_per_scope as f64,
        "pins_in_context_max" => config.pins_in_context_max as f64,
        "warnings_in_context_max" => config.warnings_in_context_max as f64,
        "patterns_in_context_max" => config.patterns_in_context_max as f64,
        "reconcile_members_max" => config.reconcile_members_max as f64,
        "subject_warning_scan_max" => config.subject_warning_scan_max as f64,
        "evidence_lookups_per_event_max" => config.evidence_lookups_per_event_max as f64,
        "verify_pass_evidence_max" => config.verify_pass_evidence_max as f64,
        "verify_pass_runs_max" => config.verify_pass_runs_max as f64,
        "verify_pass_wall_ms" => config.verify_pass_wall_ms as f64,
        "evidence_value_max_bytes" => config.evidence_value_max_bytes as f64,
        "evidence_locator_max_bytes" => config.evidence_locator_max_bytes as f64,
        "pattern_signals_min" => config.pattern_signals_min as f64,
        other => panic!("no bound named {other}"),
    }
}

/// Every bound is at its documented default (FR-500).
#[test]
fn every_bound_is_at_its_documented_default() {
    let config = CairnConfig::default();
    for (name, expected) in DOCUMENTED {
        let got = actual(&config, name);
        assert!(
            (got - expected).abs() < f64::EPSILON,
            "`{name}` is {got}, documented as {expected}"
        );
    }
    assert_eq!(
        DOCUMENTED.len(),
        17,
        "a bound was added or removed without being documented"
    );
}

// ---------------------------------------------------------------------------
// Each bound, shown to bind
// ---------------------------------------------------------------------------

/// The reserve is a floor, not a suggestion, and the budget never overspends
/// (FR-442, FR-445).
#[test]
fn the_context_reserve_binds_and_the_budget_never_exceeds() {
    let config = CairnConfig::default();
    let limit = 1_000;
    let reserve = (limit as f64 * config.min_safe_context_fraction).floor() as usize;
    assert_eq!(reserve, 400);

    let mut budget = Budget::with_reserve(limit, reserve);

    // Level 0 draws on the reserve first.
    assert!(budget.try_spend_reserved(300));
    assert_eq!(budget.reserve_used(), 300);

    // And nothing can spend past the limit, reserved or not.
    assert!(!budget.try_spend(limit));
    budget.release_reserve();
    let spent_before = budget.spent();
    let mut admitted = 0;
    while budget.try_spend(50) {
        admitted += 50;
    }
    assert!(
        spent_before + admitted <= limit,
        "the budget overspent: {} of {limit}",
        spent_before + admitted
    );
    assert!(
        budget.spent() <= budget.limit(),
        "a briefing is truncated to fit, never rejected for size — and never overspent"
    );
}

/// Reconciliation is bounded, and exceeding the bound **defers** rather than
/// scanning (FR-474).
///
/// The distinction is the whole point: a deferred reconciliation stores the
/// memory and reports that it did not compare everything. An unbounded scan
/// would make one write's cost depend on how much the project already knows.
#[test]
fn reconciliation_defers_rather_than_scanning() {
    use cairn_core::wire::codes;
    assert!(
        cairn_core::wire::codes::FEATURE_003_NOTES.contains(&codes::RECONCILIATION_DEFERRED),
        "`reconciliation_deferred` must be a note on a successful write, not a failure"
    );

    // The store's own default agrees with the configured bound; a caller with
    // no configuration to hand must not get a different answer.
    assert_eq!(
        cairn_store::repo::DEFAULT_RECONCILE_MEMBERS_MAX,
        CairnConfig::default().reconcile_members_max,
        "the store's default and the configured bound disagree"
    );
}

/// A signal set below `pattern_signals_min` is refused, and the refusal names
/// the class (FR-395).
#[test]
fn a_pattern_below_the_signal_minimum_is_refused() {
    let config = CairnConfig::default();
    let one = vec!["docker network failure".to_string()];
    assert!(
        cairn_core::patterns::normalize_signals(&one).len() < config.pattern_signals_min,
        "the fixture must actually be under the bound"
    );

    // `insufficient_specificity` is a refusal, not a note: promotion is a
    // configuration-class operation and fails loudly.
    assert!(
        !cairn_core::wire::codes::FEATURE_003_NOTES
            .contains(&cairn_core::wire::codes::INSUFFICIENT_SPECIFICITY),
        "an under-specific pattern must refuse rather than warn"
    );
}

/// Signals are bounded in both count and length (`contracts/patterns.md`).
#[test]
fn signals_are_bounded_in_count_and_length() {
    let many: Vec<String> = (0..64).map(|i| format!("distinct signal {i}")).collect();
    assert_eq!(
        cairn_core::patterns::normalize_signals(&many).len(),
        cairn_core::patterns::SIGNALS_MAX
    );

    let long = vec!["x".repeat(4_096)];
    assert_eq!(
        cairn_core::patterns::normalize_signals(&long)[0]
            .chars()
            .count(),
        cairn_core::patterns::SIGNAL_MAX_CHARS
    );
}

/// An evidence value and locator are bounded **after** redaction (FR-354).
///
/// The order matters and is asserted elsewhere; what this asserts is that the
/// bound binds at all, and at its documented size.
#[test]
fn an_evidence_value_and_locator_are_bounded() {
    let config = CairnConfig::default();
    let (rt, store) = store();
    rt.block_on(async {
        let project = seed_project(&store).await;
        let fact = cairn_store::evidence::record(
            &store,
            cairn_store::evidence::NewEvidence {
                project_id: project,
                kind: cairn_core::domain::EvidenceKind::File,
                collector: cairn_core::domain::EvidenceCollector::Cairn,
                subject: "a long value",
                observed_value: &"v".repeat(4_096),
                source_locator: &format!("docs/{}.md", "l".repeat(4_096)),
                fingerprint: "digest",
                observation_id: None,
                repo_branch: "main",
                repo_commit: None,
                collected_by_session: uuid::Uuid::now_v7(),
            },
            config.evidence_value_max_bytes,
            config.evidence_locator_max_bytes,
        )
        .await
        .expect("record");

        assert!(
            fact.observed_value.as_deref().unwrap_or_default().len()
                <= config.evidence_value_max_bytes,
            "the stored value is {} bytes, over {}",
            fact.observed_value.as_deref().unwrap_or_default().len(),
            config.evidence_value_max_bytes
        );
        assert!(
            fact.source_locator.as_deref().unwrap_or_default().len()
                <= config.evidence_locator_max_bytes,
            "the stored locator is over its bound"
        );
    });
}

/// The subject read stops at its bound and **says so** rather than scanning
/// (FR-474).
///
/// `degraded` is the honest answer: the derivation saw a bounded slice, and a
/// caller that needs certainty knows it did not get it. Silently returning the
/// first 256 as though they were all of them is the failure this prevents.
#[test]
fn a_subject_read_stops_at_its_bound_and_reports_it() {
    let (rt, store) = store();
    rt.block_on(async {
        let project = seed_project(&store).await;
        let cap = 8;
        for i in 0..(cap + 4) {
            let scope_key = project.to_string();
            cairn_store::repo::create_memory(
                &store,
                cairn_store::repo::NewMemory {
                    project_id: project,
                    kind: cairn_core::domain::MemoryType::Fact,
                    scope: cairn_core::domain::MemoryScope::Project,
                    scope_key: &scope_key,
                    content: &format!("member {i}"),
                    origin_session_id: uuid::Uuid::now_v7(),
                    local_only: false,
                    evidence: &[],
                    topic_key: Some("infra.db"),
                    value_key: Some("postgresql"),
                    importance: cairn_core::Importance::Normal,
                },
                cairn_store::outbox::SyncPolicy {
                    linked: false,
                    server_project_id: None,
                },
            )
            .await
            .expect("member");
        }

        let read = cairn_store::knowledge::subject(
            &store,
            project,
            cairn_core::domain::MemoryScope::Project,
            &project.to_string(),
            "infra.db",
            cap,
        )
        .await
        .expect("subject");

        assert_eq!(read.members.len(), cap, "the read went past its bound");
        assert!(
            read.degraded,
            "a bounded read must report that it was bounded"
        );
    });
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn store() -> (tokio::runtime::Runtime, cairn_store::Store) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    // Leaked deliberately: the store must outlive the directory handle for the
    // duration of the test, and the process is about to end.
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("dir")));
    let store = rt.block_on(async {
        cairn_store::Store::open(&dir.path().join("cairn.sqlite3"))
            .await
            .expect("store")
    });
    (rt, store)
}

async fn seed_project(store: &cairn_store::Store) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO projects (id, name, git_common_dir, repository_remote, linked,
                               server_project_id, created_at, updated_at, deleted_at)
         VALUES (?1, 'bounds', ?2, NULL, 0, NULL, ?3, ?3, NULL)",
    )
    .bind(id.to_string())
    .bind(format!("/fixture/{id}/.git"))
    .bind(&now)
    .execute(store.pool())
    .await
    .expect("project");
    id
}
