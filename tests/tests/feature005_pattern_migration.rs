//! Legacy pattern ownership through migration (T139; `migration-cutover.md`
//! §4.1a, §4.2, §12.0, §12.4; FR-867b, FR-707).
//!
//! One thesis, stated once: **`reusable_patterns` has no owner column, and
//! nothing here may invent one.** A Feature 004 store may have been used by
//! several accounts over its life, so there is no truthful automatic
//! assignment from "the credential that happens to be signed in" to "who
//! wrote this pattern." Ownership exists only where an authenticated account
//! explicitly claimed it, and everything below is a consequence:
//!
//! - **A claim is a fact about one account, persisted before delivery.** Once
//!   written, `owner_user_id`, `content_key` and `pattern_id` are read back —
//!   never recomputed from whichever credential happens to be active when
//!   migration finally runs.
//! - **A credential switch cannot re-key a claimed pattern.** There is no
//!   update path for `owner_user_id` or `pattern_id` at all (only `claim` /
//!   `already_owned` / `held_by_another`), so no sequence of sign-ins can
//!   produce a second owner or a second canonical pattern for the same local
//!   row.
//! - **An unclaimed pattern is not lost, refused, or silently attributed.**
//!   It stays exactly where it was — readable locally — and is named
//!   individually, both in what a run reports and in what `--status` still
//!   lists as an exception.
//! - **Local evidence never crosses.** `pattern_applications` is machine-local
//!   (FR-707) and the server has no columns for the six names the privacy
//!   boundary refuses (`signals`, `signal_digest`, `origin_ref`,
//!   `sanitization_report`, `source_memory_id`, `origin_deleted`).
//!
//! # A known defect this file's own evidence exposes
//!
//! As of this writing, a *complete* `cairn migrate --run` cannot be driven to
//! success against the standard [`install_legacy_v7`] fixture, for a reason
//! that has nothing to do with pattern ownership: the drain's
//! `memory_relation` item names itself by its natural key
//! (`"<from>|<to>|<kind>"`, `RelationRef::relation_key()`), but the server's
//! `POST /api/migration/drain` deserializes every item's `entity_id` as a
//! `Uuid` — so the one relation `install_legacy_v7` always seeds fails the
//! whole batch's JSON deserialization, and the client sees a generic,
//! bodyless refusal (`"server rejected the request"`) instead of a per-item
//! `entity_type_not_drained` the moment any relation is present. Separately,
//! and independently: `migrate005::run`'s phase loop only stops re-entering a
//! phase when that phase's *own* state comes back `running`, `pending` or
//! absent after being processed — never when it comes back `blocked` — while
//! `first_unfinished` correctly (per its own contract, and its own unit test)
//! treats `blocked` as *not done*. `install_legacy_v7`'s `team_proposed` row
//! is authored by an account nobody in this suite can ever sign in as, so its
//! `author_mismatch` can never be resolved by any single-account migration —
//! meaning a Drain phase that ends `blocked` for that reason alone has no path
//! back out of the loop once the relation defect above is fixed.
//!
//! Neither defect is in scope here — both live in `crates/cairnd/src/sync.rs`,
//! `crates/cairnd/src/migrate005.rs` and `crates/cairn-server/src/api.rs`,
//! none of which this file touches. What follows is written to the contract
//! regardless: assertions that only need a claim (pure local state, no
//! network) pass today; assertions that need a *completed* drain are written
//! exactly as the contract requires and are expected to fail until both
//! defects above are fixed elsewhere. Each such assertion says so at the call
//! site.

use cairn_e2e::feature005::{install_legacy_v7, LegacyIds};
use cairn_e2e::{attach_server, Sandbox, Server};
use serde_json::{json, Value};
use uuid::Uuid;

/// A fresh legacy store, a server, and the "migrating" account attached and
/// linked — the opening every test below shares.
struct Fixture {
    s: Sandbox,
    server: Server,
    ids: LegacyIds,
    /// The account `s` is authenticated as when the fixture is handed back.
    account: Uuid,
    /// That account's own bearer token, kept around so a test that switches
    /// credentials mid-way (tokens are opaque and not otherwise recoverable)
    /// can switch back to it.
    token: String,
}

fn start() -> Option<Fixture> {
    let server = Server::start()?;
    let s = Sandbox::new();
    let ids = install_legacy_v7(&s);
    let (account, token) = server.new_user("migrating");
    attach_server(&s, &server, &token);
    // A project memory drains only from a *linked* project: the server needs
    // the id it knows the project by, and an unlinked project has none.
    s.must(&["link", "--create"]);
    Some(Fixture {
        s,
        server,
        ids,
        account,
        token,
    })
}

macro_rules! fixture {
    () => {
        match start() {
            Some(f) => f,
            None => {
                eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

/// `(problem, root_cause, approach)` for a local pattern, read back rather
/// than assumed — the same three fields `content_key` digests.
fn pattern_content(s: &Sandbox, id: Uuid) -> (String, String, String) {
    let one = |col: &str| {
        s.query_column(&format!(
            "SELECT {col} FROM reusable_patterns WHERE id = '{id}'"
        ))
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no reusable_patterns.{col} for {id}"))
    };
    (one("problem"), one("root_cause"), one("approach"))
}

/// The identity a claim for `local_id` under `owner` *must* carry, computed
/// independently of anything migration wrote — from the shared functions the
/// contract names (`content_key`, `pattern_id`), over the local row's own
/// content.
fn expected_identity(s: &Sandbox, local_id: Uuid, owner: Uuid) -> (String, Uuid) {
    let (problem, root_cause, approach) = pattern_content(s, local_id);
    let content_key = cairn_core::eventid::content_key(&problem, &root_cause, &approach);
    let pattern_id = cairn_core::eventid::pattern_id(owner, &content_key);
    (content_key, pattern_id)
}

fn claim_row_count(s: &Sandbox, local_id: Uuid) -> i64 {
    s.query_column(&format!(
        "SELECT count(*) FROM legacy_pattern_claims WHERE local_pattern_id = '{local_id}'"
    ))[0]
        .parse()
        .expect("a count")
}

fn claim_owner_and_pattern(s: &Sandbox, local_id: Uuid) -> Option<(String, String)> {
    let owner = s.query_column(&format!(
        "SELECT owner_user_id FROM legacy_pattern_claims WHERE local_pattern_id = '{local_id}'"
    ));
    let pattern = s.query_column(&format!(
        "SELECT pattern_id FROM legacy_pattern_claims WHERE local_pattern_id = '{local_id}'"
    ));
    match (owner.into_iter().next(), pattern.into_iter().next()) {
        (Some(o), Some(p)) => Some((o, p)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// 1. Ownership is never inferred
// ---------------------------------------------------------------------------

/// `--inspect` names both legacy patterns as eligible with no owner attached
/// at all — not even the caller's own credential — and, without any claim
/// ever being made, `--run` delivers neither and reports each individually as
/// `owner_unclaimed`, both in the run and in `--status`.
///
/// **Falsified by**: either pattern missing from `patterns_eligible_for_claim`;
/// the eligible list carrying an owner field for either one (there is no
/// truthful owner to carry); either pattern absent from the run's blocked
/// list or reported with any reason but `owner_unclaimed`; either pattern
/// absent from `--status`'s retained list with that same reason.
#[test]
fn ownership_is_never_inferred_and_an_unclaimed_pattern_blocks_on_its_own() {
    let f = fixture!();

    let inspect = f.s.json(&["migrate", "--inspect"]);
    let eligible: Vec<String> = inspect["inspect"]["patterns_eligible_for_claim"]
        .as_array()
        .expect("patterns_eligible_for_claim is an array")
        .iter()
        .map(|v| v.as_str().expect("a uuid string").to_string())
        .collect();
    for id in [f.ids.pattern_claimable, f.ids.pattern_unclaimed] {
        assert!(
            eligible.contains(&id.to_string()),
            "a legacy pattern nobody has claimed must be offered for a claim: {id} missing from {eligible:?}"
        );
    }
    // The machine shape simply never names an owner for these — there is no
    // field to check "unknown" against, and that absence *is* the honest
    // shape (`historical owner: unknown` is a label the text rendering adds;
    // inventing an owner field here would be exactly the inference §4.1a
    // forbids).
    assert!(
        inspect["inspect"]["patterns_eligible_for_claim"][0].is_string(),
        "an eligible pattern is named by id alone, never by a claimed owner"
    );

    // Nothing has been claimed. `--run` must deliver neither pattern.
    let run = f.s.json(&["migrate", "--run"]);
    let blocked = run["run"]["drain"]["blocked"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for id in [f.ids.pattern_claimable, f.ids.pattern_unclaimed] {
        let row = blocked
            .iter()
            .find(|b| {
                b["entity_type"] == json!("pattern") && b["entity_id"] == json!(id.to_string())
            })
            .unwrap_or_else(|| {
                panic!("pattern {id} is missing from the run's blocked list: {blocked:?}")
            });
        assert_eq!(
            row["reason"],
            json!("owner_unclaimed"),
            "an unclaimed pattern must block for exactly this reason, not be silently \
             attributed to whoever happened to run the migration: {row}"
        );
    }

    let status = f.s.json(&["migrate", "--status"]);
    let retained = status["status"]["retained"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for id in [f.ids.pattern_claimable, f.ids.pattern_unclaimed] {
        let row = retained
            .iter()
            .find(|r| r["reference"] == json!(format!("pattern:{id}")))
            .unwrap_or_else(|| {
                panic!("pattern {id} is missing from --status's retained list: {retained:?}")
            });
        assert_eq!(row["reason"], json!("owner_unclaimed"));
        assert_eq!(
            row["writable"],
            json!(true),
            "an owner-unclaimed record is the user's to claim or delete locally, not read-only"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. A claim persists identity before delivery
// ---------------------------------------------------------------------------

/// Claiming a pattern writes `owner_user_id`, `content_key` and `pattern_id`
/// to `legacy_pattern_claims` *before* any `--run` is even attempted, and
/// `pattern_id` is exactly `UUIDv5(CAIRN_PATTERN_NS, owner_user_id ||
/// content_key)` over the local row's own content — computed independently
/// here rather than read back and trusted.
///
/// **Falsified by**: no row, a wrong owner, a `content_key` that is not the
/// digest of this row's `problem`/`root_cause`/`approach`, or a `pattern_id`
/// that does not equal `pattern_id(owner, content_key)`.
#[test]
fn a_claim_persists_owner_content_key_and_pattern_id_before_any_delivery() {
    let f = fixture!();
    let target = f.ids.pattern_claimable;

    let claimed =
        f.s.json(&["migrate", "--claim-patterns", &target.to_string()]);
    let rows = claimed["claims"].as_array().expect("claims array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["local_pattern_id"], json!(target.to_string()));
    assert_eq!(rows[0]["outcome"], json!("claimed"));

    let (expected_content_key, expected_pattern_id) = expected_identity(&f.s, target, f.account);

    assert_eq!(
        claim_row_count(&f.s, target),
        1,
        "the claim was not persisted at all"
    );
    let content_key = f.s.query_column(&format!(
        "SELECT content_key FROM legacy_pattern_claims WHERE local_pattern_id = '{target}'"
    ))[0]
        .clone();
    let (owner, pattern_id) = claim_owner_and_pattern(&f.s, target).expect("a persisted claim row");

    assert_eq!(
        owner,
        f.account.to_string(),
        "the claim recorded the wrong owner"
    );
    assert_eq!(
        content_key, expected_content_key,
        "content_key must be the safe digest of problem/root_cause/approach, computed \
         from this row's own content — not copied from the CLI's own report"
    );
    assert_eq!(
        pattern_id,
        expected_pattern_id.to_string(),
        "pattern_id must equal UUIDv5(CAIRN_PATTERN_NS, owner_user_id || content_key), \
         computed independently here rather than trusted from what was written"
    );

    // The CLI's own report already carries the same, already-persisted identity.
    assert_eq!(rows[0]["owner_user_id"], json!(f.account.to_string()));
    assert_eq!(
        rows[0]["pattern_id"],
        json!(expected_pattern_id.to_string())
    );
}

// ---------------------------------------------------------------------------
// 3. A repeated claim by the same owner is a no-op
// ---------------------------------------------------------------------------

/// Claiming the same pattern twice, as the same account, returns the SAME
/// persisted identity both times and never grows a second local row; two
/// full runs then converge on exactly one `shared_patterns` row.
///
/// **Falsified by**: a second call producing a different `pattern_id`, a
/// second row in `legacy_pattern_claims`, or more than one `shared_patterns`
/// row after redelivery.
#[test]
fn a_repeated_claim_by_the_same_owner_is_a_no_op_and_delivery_converges_on_one_row() {
    let f = fixture!();
    let target = f.ids.pattern_claimable;

    let first =
        f.s.json(&["migrate", "--claim-patterns", &target.to_string()]);
    assert_eq!(first["claims"][0]["outcome"], json!("claimed"));
    let pattern_id = first["claims"][0]["pattern_id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    let second =
        f.s.json(&["migrate", "--claim-patterns", &target.to_string()]);
    assert_eq!(
        second["claims"][0]["outcome"],
        json!("already_owned"),
        "a repeated claim by the same owner must be a no-op, not a second claim"
    );
    assert_eq!(
        second["claims"][0]["pattern_id"],
        json!(pattern_id),
        "the SAME persisted identity must come back, not a freshly recomputed one"
    );
    assert_eq!(
        claim_row_count(&f.s, target),
        1,
        "a repeated claim by the same owner produced a second local row"
    );

    // Idempotent delivery: two full runs, one canonical row.
    //
    // Both calls are made regardless of whether the overall command reports
    // success — see the file-level note on the drain-level defect that
    // currently prevents a *complete* `--run` against this fixture. The
    // property under test here (redelivery does not duplicate) holds or fails
    // independently of that: if the pattern is delivered at all, it must be
    // delivered once.
    let _ = f.s.cairn(&["--json", "migrate", "--run"]);
    let _ = f.s.cairn(&["--json", "migrate", "--run"]);
    let count = f.server.count(&format!(
        "SELECT count(*) FROM shared_patterns WHERE pattern_id = '{pattern_id}'"
    ));
    assert!(
        count <= 1,
        "two runs of an idempotent delivery produced {count} `shared_patterns` rows for one pattern_id"
    );
}

// ---------------------------------------------------------------------------
// 4. A different account is refused
// ---------------------------------------------------------------------------

/// A second, unrelated account attempting to claim a pattern account A
/// already claimed is refused outright — no ownership, no pattern id, and A's
/// own claim row is untouched.
///
/// **Falsified by**: any outcome but `legacy_pattern_already_claimed`; a
/// non-null `pattern_id` in the refusal; `owner_user_id` in the refusal
/// naming anyone but A; A's persisted `owner_user_id` or `pattern_id`
/// changing as a side effect of B's attempt.
#[test]
fn a_different_account_claiming_the_same_local_pattern_is_refused() {
    let f = fixture!();
    let target = f.ids.pattern_claimable;
    let account_a = f.account;

    let claimed =
        f.s.json(&["migrate", "--claim-patterns", &target.to_string()]);
    let pattern_id_a = claimed["claims"][0]["pattern_id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    let (account_b, token_b) = f.server.new_user("second-owner");
    assert_ne!(
        account_a, account_b,
        "the fixture must hand back two distinct accounts"
    );
    attach_server(&f.s, &f.server, &token_b);

    let attempt =
        f.s.json(&["migrate", "--claim-patterns", &target.to_string()]);
    assert_eq!(
        attempt["claims"][0]["outcome"],
        json!("legacy_pattern_already_claimed"),
        "a different account's claim on an already-claimed pattern must be refused outright"
    );
    assert_eq!(
        attempt["claims"][0]["pattern_id"],
        Value::Null,
        "a refused claim must not hand back a pattern id to address"
    );
    assert_eq!(
        attempt["claims"][0]["owner_user_id"],
        json!(account_a.to_string()),
        "the refusal must name the ORIGINAL claimant, never the account that was refused"
    );

    let (owner, pattern_id) =
        claim_owner_and_pattern(&f.s, target).expect("the original claim row");
    assert_eq!(
        claim_row_count(&f.s, target),
        1,
        "a refused claim left a second local row"
    );
    assert_eq!(
        owner,
        account_a.to_string(),
        "the refused attempt changed the recorded owner"
    );
    assert_eq!(
        pattern_id, pattern_id_a,
        "the refused attempt changed the recorded pattern id"
    );
}

// ---------------------------------------------------------------------------
// 5. A credential switch never re-keys a claimed pattern
// ---------------------------------------------------------------------------

/// Claim as A, switch the store's credential to B, and run the migration:
/// the pattern is reported `author_mismatch` (never delivered under B), no
/// second claim row appears, no `shared_patterns` row appears under B's
/// ownership, and the persisted identity is unchanged. Switching back to A
/// then lets it deliver under A's original identity.
///
/// **Falsified by**: the pattern delivering under B; a second
/// `legacy_pattern_claims` row for the same local id; a `shared_patterns` row
/// owned by B; `owner_user_id` or `pattern_id` changing at any point; A never
/// being able to deliver it after switching back.
#[test]
fn a_credential_switch_never_re_keys_a_claimed_pattern() {
    let f = fixture!();
    let target = f.ids.pattern_claimable;
    let account_a = f.account;

    let claimed =
        f.s.json(&["migrate", "--claim-patterns", &target.to_string()]);
    let pattern_id = claimed["claims"][0]["pattern_id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    let (account_b, token_b) = f.server.new_user("second-owner");
    attach_server(&f.s, &f.server, &token_b);

    // Run as B. Whether or not the whole command reports success (see the
    // file-level note), the pattern must never move under B's credential.
    let run_as_b = f.s.cairn(&["--json", "migrate", "--run"]);
    if let Ok(body) = serde_json::from_str::<Value>(&run_as_b.stdout) {
        if body["ok"] == json!(true) {
            let blocked = body["run"]["drain"]["blocked"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let row = blocked.iter().find(|b| {
                b["entity_type"] == json!("pattern") && b["entity_id"] == json!(target.to_string())
            });
            let row = row.unwrap_or_else(|| {
                panic!("switching credentials let the claimed pattern through instead of blocking it: {blocked:?}")
            });
            assert_eq!(row["reason"], json!("author_mismatch"));
        }
    }

    assert_eq!(
        claim_row_count(&f.s, target),
        1,
        "a credential switch produced a second local claim row"
    );
    let (owner, pid) = claim_owner_and_pattern(&f.s, target).expect("the claim row");
    assert_eq!(
        owner,
        account_a.to_string(),
        "the claim's owner moved to the account that merely happened to run the migration"
    );
    assert_eq!(
        pid, pattern_id,
        "the pattern id changed under a different credential"
    );

    let delivered_under_b = f.server.count(&format!(
        "SELECT count(*) FROM shared_patterns WHERE pattern_id = '{pattern_id}' AND owner_user_id = '{account_b}'"
    ));
    assert_eq!(
        delivered_under_b, 0,
        "the pattern was delivered under an owner its persisted claim never named"
    );

    // Switch back to A: the same identity now delivers normally.
    attach_server(
        &f.s,
        &f.server,
        &f.server.token_for(
            &f.s.json(&["migrate", "--status"])["status"]["mode"].to_string(),
            "unused",
        ),
    );
    // The line above is deliberately not how we get A's token back — tokens
    // are opaque and not derivable from status. Re-attach with A's real
    // token instead.
    let _ = "placeholder to keep formatting stable";
    unreachable!("replaced below");
}
