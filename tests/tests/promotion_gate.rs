//! The promotion gate as a whole, exercised against a seeded adversarial
//! corpus (FR-506–FR-520, SC-421, SC-424a).
//!
//! The individual checks are unit-tested beside the code in
//! `crates/cairn-core/src/promotion.rs`, one test per check. What this file
//! adds is the corpus: every one of the nine rejection classes driven through
//! the gate, plus the applicability cases FR-514 and FR-578 govern.
//!
//! The gate is pure, so none of this needs a store. That is the point of it
//! being pure — 003's `privacy_promotion` established the pattern and this
//! reuses it rather than inventing a fixture.

use cairn_core::domain::{ApplicabilityKind, MemoryState, PromotionTarget};
use cairn_core::promotion::evaluate_promotion;
use cairn_core::validate::{ProjectIdentity, CONTENT_CLASSES, INVALID_APPLICABILITY};
use uuid::Uuid;

const SALT: &str = "test-machine-salt";
const PROJECT: &str = "acme-widgets";

fn promote(
    content: &str,
    applicability: &[(ApplicabilityKind, String)],
) -> Result<cairn_core::promotion::PromotionApproval, cairn_core::promotion::PromotionRejection> {
    evaluate_promotion(
        content,
        Some("build.cache_dir"),
        Some("clears_on_stale"),
        applicability,
        std::slice::from_ref(&ProjectIdentity(PROJECT.to_string())),
        Some(Uuid::now_v7()),
        Some(SALT),
        PromotionTarget::Personal,
        true,
        Some(MemoryState::Active),
    )
}

/// SC-421 — the seeded adversarial corpus, covering **all nine** classes.
///
/// The corpus is checked against the validator's declared class list, so a
/// class added without a corpus entry fails this test rather than leaving the
/// criterion silently unverified. That coupling is the whole reason SC-421 was
/// rewritten during remediation: it used to name six classes while FR-546
/// declared nine, and both were cited by the same task, so every
/// cross-reference resolved while three classes had no case at all.
#[test]
fn the_adversarial_corpus_covers_every_class_and_each_is_refused_by_name() {
    let corpus: &[(&str, &str)] = &[
        ("absolute_path", "The cache lives at /var/tmp/acme/build"),
        ("home_dir_ref", "It is under ~/Library/Caches/build"),
        ("drive_letter_path", r"On Windows it is D:\build\cache"),
        ("file_uri", "Configured by file:///opt/conf/build.toml"),
        (
            "credentialed_url",
            "Pull from https://ci:token@mirror.example/cache",
        ),
        ("env_assignment", "Override with CARGO_TARGET_DIR=/tmp/x"),
        (
            "encoded_secret_shape",
            "Signed with 9f8e7d6c5b4a39281706f5e4d3c2b1a0",
        ),
        ("project_identifying", "The acme-widgets cache is large"),
        ("command_shaped", "Clear it with rm -rf ./target"),
    ];

    let covered: Vec<&str> = corpus.iter().map(|(class, _)| *class).collect();
    for class in CONTENT_CLASSES {
        assert!(
            covered.contains(class),
            "class {class:?} is declared by the validator and has no corpus case (SC-421)"
        );
    }

    for (class, content) in corpus {
        let rejection = promote(content, &[]).expect_err(&format!("{class}: {content:?} passed"));
        assert_eq!(
            rejection.check, "shared_content_validation",
            "{content:?} was refused by the wrong check"
        );
        assert_eq!(
            rejection.class,
            Some(*class),
            "{content:?} was refused as {:?}, expected {class}",
            rejection.class
        );
        // FR-510 — no fragment of the content in the rejection, for any class.
        let rendered = format!("{rejection:?}");
        for token in content.split_whitespace().filter(|t| t.len() > 3) {
            assert!(
                !rendered.contains(token),
                "the rejection leaked {token:?}: {rendered}"
            );
        }
    }
}

/// The corpus proves refusals. This proves the gate is not simply refusing
/// everything, which a refusal-only suite could not distinguish.
#[test]
fn content_worth_promoting_is_approved() {
    let approved = promote(
        "Clear the build cache when a stale artifact is suspected",
        &[(ApplicabilityKind::Tool, "cargo".to_string())],
    )
    .expect("ordinary guidance must promote");
    assert_eq!(
        approved.sanitized_applicability,
        vec![(ApplicabilityKind::Tool, "cargo".to_string())]
    );
    assert!(!approved.origin_digest.is_empty());
}

/// T006 / FR-514 — an applicability value outside the closed vocabulary's
/// format causes the promotion to be **refused**, never silently dropped,
/// truncated, or stored with a null kind.
///
/// Asserted as a rejection. A test that only confirmed valid values are
/// accepted would pass on an implementation that discarded invalid ones, which
/// is the exact failure FR-446 names.
#[test]
fn an_applicability_value_outside_the_vocabulary_is_refused_not_dropped() {
    for bad in ["not a value", "has-dash", "path/like", "UPPER!", "", "   "] {
        let rejection = promote(
            "Clear the build cache when a stale artifact is suspected",
            &[(ApplicabilityKind::Tool, bad.to_string())],
        )
        .expect_err(&format!("applicability value {bad:?} was accepted"));
        assert_eq!(rejection.check, "shared_content_validation", "{bad:?}");
        assert_eq!(
            rejection.class,
            Some(INVALID_APPLICABILITY),
            "{bad:?} was refused as {:?}",
            rejection.class
        );
    }
}

/// FR-578 / SC-448 — an applicability value is screened as content too, not
/// only for format. A value that *reads* as a project name is refused under
/// `project_identifying`, because what is wrong with it is what it says.
#[test]
fn an_applicability_value_that_names_the_project_is_refused_as_content() {
    let rejection = promote(
        "Clear the build cache when a stale artifact is suspected",
        &[(ApplicabilityKind::Tool, PROJECT.replace('-', "_"))],
    )
    .expect_err("a project-identifying applicability value was accepted");
    assert_eq!(rejection.check, "shared_content_validation");
    assert_eq!(rejection.class, Some("project_identifying"));
}

/// A record with no applicability facts promotes, and the universal default
/// carries through the promotion path unchanged (CHK027's question, answered by
/// behaviour rather than by a restatement in the spec).
#[test]
fn a_promotion_with_no_applicability_facts_is_accepted_as_universal() {
    let approved = promote("Prefer thiserror over hand-rolled Display impls", &[])
        .expect("no applicability facts must be accepted");
    assert!(
        approved.sanitized_applicability.is_empty(),
        "no facts means universal (FR-435); the gate must not invent one"
    );
}

// ---------------------------------------------------------------------------
// Phase 5 — the validator cannot be bypassed by the non-promotion path
// (T085–T088; FR-545, FR-546, FR-548, SC-422, SC-440)
// ---------------------------------------------------------------------------

use cairn_core::domain::MemoryType;
use cairn_store::global::{create_personal, recall_personal, NewPersonalKnowledge};
use cairn_store::Store;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
}

/// T085 — content carrying an absolute path is refused on **direct personal
/// creation**, not only on promotion.
///
/// This is the bypass the design analysis found: the content check lived inside
/// the promotion gate, and direct creation never called the gate. An agent
/// reaching for `cairn_remember action=create domain=personal` — the path it is
/// most likely to take, because it needs no project memory to exist first — went
/// entirely unchecked.
#[test]
fn direct_personal_creation_is_refused_for_an_absolute_path() {
    runtime().block_on(async {
        let store = Store::open_memory().await.expect("store");
        let owner = Uuid::now_v7();
        let none: [ProjectIdentity; 0] = [];

        let refused = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner,
                MemoryType::Convention,
                "scratch files live at /Users/alice/tmp, safe to delete",
                Some("build.tmp_dir"),
                Some("alice_machine"),
                vec![],
            ),
            &none,
        )
        .await;
        assert!(
            refused.is_err(),
            "an absolute path was accepted by direct personal creation"
        );

        // The identical content is refused the identical way at promotion — one
        // shared validator, one answer (FR-579).
        let at_promotion = promote(
            "scratch files live at /Users/alice/tmp, safe to delete",
            &[],
        )
        .expect_err("promotion accepted an absolute path");
        assert_eq!(at_promotion.class, Some("absolute_path"));
    });
}

/// T086 / SC-440 — a refused creation leaves **nothing** behind.
///
/// No record, no partial record, no queued outbox entry. Inspected across all
/// three, because "the insert failed" and "nothing was written" are different
/// claims: a two-statement path could refuse the second and leave the first.
#[test]
fn a_refused_creation_leaves_no_record_and_no_outbox_entry() {
    runtime().block_on(async {
        let store = Store::open_memory().await.expect("store");
        let owner = Uuid::now_v7();
        let none: [ProjectIdentity; 0] = [];

        let _ = create_personal(
            &store,
            NewPersonalKnowledge::direct(
                owner,
                MemoryType::Convention,
                "the token is 4f2a1c9e8b7d6a5f4e3d2c1b0a998877 and rotates monthly",
                Some("auth.token"),
                Some("monthly"),
                vec![],
            ),
            &none,
        )
        .await;

        let records = recall_personal(&store, owner, None, None, &[], 50)
            .await
            .expect("recall");
        assert!(records.is_empty(), "a refused creation stored a record");

        let personal_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM personal_knowledge")
            .fetch_one(store.pool())
            .await
            .expect("count");
        assert_eq!(personal_rows, 0, "a refused creation left a partial row");

        let queued: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox WHERE entity_type = 'personal_knowledge'",
        )
        .fetch_one(store.pool())
        .await
        .expect("count");
        assert_eq!(queued, 0, "a refused creation queued an outbox entry");
    });
}

/// T088 / SC-422 — a promoted record carries **no verification field of any
/// kind**, in the stored form or the serialized one.
///
/// Not "no authority above `attested`" — that still admits a field holding
/// `attested`, and a field holding `attested` is a place for one project's
/// deterministic check to become a project-independent claim one migration
/// later. There is nothing to reset because there is nowhere to hold a value.
#[test]
fn a_promoted_record_carries_no_verification_field_in_either_form() {
    runtime().block_on(async {
        let store = Store::open_memory().await.expect("store");
        let owner = Uuid::now_v7();
        let none: [ProjectIdentity; 0] = [];

        let mut record = NewPersonalKnowledge::direct(
            owner,
            MemoryType::Convention,
            "clear the build cache when a stale artifact is suspected",
            Some("build.cache_dir"),
            Some("clears_on_stale"),
            vec![],
        );
        // The one thing promotion adds.
        record.origin_digest = Some("a-digest-that-must-stay-local".to_string());

        let outcome = create_personal(&store, record, &none)
            .await
            .expect("create");

        let serialized = serde_json::to_string(&outcome.record).expect("serialize");
        for forbidden in [
            "verification",
            "verified",
            "attested",
            "authority",
            "last_verified",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "a promoted record serialized `{forbidden}`: {serialized}"
            );
        }
        // The stored form, field by field.
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('personal_knowledge')")
                .fetch_all(store.pool())
                .await
                .expect("columns");
        for forbidden in ["verification", "verification_authority", "last_verified_at"] {
            assert!(
                !columns.iter().any(|c| c == forbidden),
                "personal_knowledge has a `{forbidden}` column: {columns:?}"
            );
        }
        // And the digest promotion added is local-only (FR-551).
        assert!(
            !serialized.contains("a-digest-that-must-stay-local"),
            "the origin digest reached the serialized form: {serialized}"
        );
    });
}

// ---------------------------------------------------------------------------
// T086 — the real promotion path, refusing, naming the check
// ---------------------------------------------------------------------------

/// A refused promotion through the **actual** daemon path names the check that
/// failed, and leaves no record and no outbox entry.
///
/// The tests above drive `evaluate_promotion` directly, which is right for
/// isolating one check at a time — but a gate that is never called is a gate that
/// passes every unit test. This exercises the path a real caller takes, so
/// removing the `evaluate_promotion` call from `crates/cairnd/src/promote.rs`
/// fails here rather than nowhere.
///
/// The check's *name* matters as much as the refusal: an agent told only
/// "refused" cannot tell "reword this" from "this memory is superseded" from
/// "you are not a member", and those call for three different next actions.
#[test]
fn a_refused_promotion_through_the_daemon_names_its_check_and_writes_nothing() {
    let s = cairn_e2e::Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    let cwd = s.repo_path().to_string_lossy().to_string();

    // A source whose content trips the validator at check 1.
    let source = s.json(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "--topic-key",
        "build.cache",
        "--value-key",
        "clear_on_stale",
        "clear the cache at /Users/dev/src/thing/target when a stale artifact is suspected",
    ]);
    let source_id = source["memory"]["id"].as_str().expect("memory id");

    let before = counts(&s);
    let refused = mcp.tool_result(
        "cairn_remember",
        serde_json::json!({
            "action": "promote",
            "target": "personal",
            "memory_id": source_id,
        }),
        &cwd,
    );
    assert_eq!(
        refused["isError"], true,
        "the promotion was accepted: {refused}"
    );
    let message = refused["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        message.contains("shared_content_validation"),
        "the refusal does not name the check that failed: {message}"
    );
    assert!(
        message.contains("absolute_path"),
        "the refusal does not name the rejection class: {message}"
    );
    assert!(
        !message.contains("/Users/dev"),
        "the refusal echoed the offending content: {message}"
    );

    assert_eq!(
        counts(&s),
        before,
        "a refused promotion left a record or an outbox entry behind"
    );

    // A source with **no subject** trips a different check, and reports that one
    // rather than the first — the gate's order is fixed and the first failure is
    // the one named.
    let no_subject = s.json(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "a perfectly clean claim with no subject key",
    ]);
    let refused = mcp.tool_result(
        "cairn_remember",
        serde_json::json!({
            "action": "promote",
            "target": "personal",
            "memory_id": no_subject["memory"]["id"].as_str().expect("id"),
        }),
        &cwd,
    );
    assert_eq!(
        refused["isError"], true,
        "the promotion was accepted: {refused}"
    );
    let message = refused["content"][0]["text"].as_str().unwrap_or_default();
    assert!(
        message.contains("no_subject"),
        "a source with no subject key was refused for the wrong reason: {message}"
    );
}

/// The three counts a refused promotion must not move.
fn counts(s: &cairn_e2e::Sandbox) -> Vec<String> {
    s.query_column(
        "SELECT (SELECT COUNT(*) FROM personal_knowledge)
             || ':' || (SELECT COUNT(*) FROM team_knowledge)
             || ':' || (SELECT COUNT(*) FROM outbox)",
    )
}
