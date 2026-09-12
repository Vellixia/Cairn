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
//!   update path for `owner_user_id` or `pattern_id` at all (only `claimed` /
//!   `already_owned` / `legacy_pattern_already_claimed`), so no sequence of
//!   sign-ins can produce a second owner or a second canonical pattern for
//!   the same local row.
//! - **An unclaimed pattern is not lost, refused, or silently attributed.**
//!   It stays exactly where it was — readable locally — and is named
//!   individually, both in what a run reports and in what `--status` still
//!   lists as an exception.
//! - **Local evidence never crosses.** `pattern_applications` is machine-local
//!   (FR-707) and the server has no columns for the six names the privacy
//!   boundary refuses (`signals`, `signal_digest`, `origin_ref`,
//!   `sanitization_report`, `source_memory_id`, `origin_deleted`).
//!
//! One PostgreSQL database serves the whole suite (every `Server::start()`
//! shares it), so every server-side count below is scoped to a `pattern_id`
//! that is itself unique to the test — `UUIDv5(account, content_key)` over a
//! freshly minted account — rather than to a bare, suite-wide `count(*)`.

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
    /// Also the account `install_legacy_v7` seeded the personal/team rows
    /// under, exactly as a real Feature 004 store's author would be.
    account: Uuid,
    /// That account's own bearer token, kept around so a test that switches
    /// credentials mid-way (tokens are opaque and not otherwise recoverable)
    /// can switch back to it.
    token: String,
}

fn start() -> Option<Fixture> {
    let server = Server::start()?;
    let s = Sandbox::new();
    let (account, token) = server.new_user("migrating");
    let ids = install_legacy_v7(&s, account);
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
        "SELECT CAST(count(*) AS TEXT) FROM legacy_pattern_claims WHERE local_pattern_id = '{local_id}'"
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
    let mine = rows
        .iter()
        .find(|r| r["local_pattern_id"] == json!(target.to_string()))
        .expect("the claimed pattern's own row");
    assert_eq!(mine["outcome"], json!("claimed"));

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
    assert_eq!(mine["owner_user_id"], json!(f.account.to_string()));
    assert_eq!(mine["pattern_id"], json!(expected_pattern_id.to_string()));
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
    let claims = first["claims"].as_array().expect("claims array");
    let mine = claims
        .iter()
        .find(|r| r["local_pattern_id"] == json!(target.to_string()))
        .expect("the claimed pattern's own row");
    assert_eq!(mine["outcome"], json!("claimed"));
    let pattern_id = mine["pattern_id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    let second =
        f.s.json(&["migrate", "--claim-patterns", &target.to_string()]);
    let claims2 = second["claims"].as_array().expect("claims array");
    let mine2 = claims2
        .iter()
        .find(|r| r["local_pattern_id"] == json!(target.to_string()))
        .expect("the claimed pattern's own row, second call");
    assert_eq!(
        mine2["outcome"],
        json!("already_owned"),
        "a repeated claim by the same owner must be a no-op, not a second claim"
    );
    assert_eq!(
        mine2["pattern_id"],
        json!(pattern_id),
        "the SAME persisted identity must come back, not a freshly recomputed one"
    );
    assert_eq!(
        claim_row_count(&f.s, target),
        1,
        "a repeated claim by the same owner produced a second local row"
    );

    // Idempotent delivery: two full runs, one canonical row.
    let _ = f.s.json(&["migrate", "--run"]);
    let _ = f.s.json(&["migrate", "--run"]);
    let count = f.server.count(&format!(
        "SELECT count(*) FROM shared_patterns WHERE pattern_id = '{pattern_id}'"
    ));
    assert_eq!(
        count, 1,
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
    let pattern_id_a = claimed["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["local_pattern_id"] == json!(target.to_string()))
        .expect("A's own claim row")["pattern_id"]
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
    let row = attempt["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["local_pattern_id"] == json!(target.to_string()))
        .expect("B's attempt against the same local id");
    assert_eq!(
        row["outcome"],
        json!("legacy_pattern_already_claimed"),
        "a different account's claim on an already-claimed pattern must be refused outright"
    );
    assert_eq!(
        row["pattern_id"],
        Value::Null,
        "a refused claim must not hand back a pattern id to address"
    );
    assert_eq!(
        row["owner_user_id"],
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
    let pattern_id = claimed["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["local_pattern_id"] == json!(target.to_string()))
        .expect("A's own claim row")["pattern_id"]
        .as_str()
        .expect("a pattern id")
        .to_string();

    let (account_b, token_b) = f.server.new_user("second-owner");
    attach_server(&f.s, &f.server, &token_b);

    // Run as B. The pattern must never move under B's credential.
    let run_as_b = f.s.json(&["migrate", "--run"]);
    let blocked = run_as_b["run"]["drain"]["blocked"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let row = blocked
        .iter()
        .find(|b| b["entity_type"] == json!("pattern") && b["entity_id"] == json!(target.to_string()))
        .unwrap_or_else(|| {
            panic!("switching credentials let the claimed pattern through instead of blocking it: {blocked:?}")
        });
    assert_eq!(row["reason"], json!("author_mismatch"));

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
    attach_server(&f.s, &f.server, &f.token);
    let _ = f.s.json(&["migrate", "--run"]);
    let delivered_under_a = f.server.count(&format!(
        "SELECT count(*) FROM shared_patterns WHERE pattern_id = '{pattern_id}' AND owner_user_id = '{account_a}'"
    ));
    assert_eq!(
        delivered_under_a, 1,
        "switching back to the original claimant must let the pattern deliver under its own, unchanged identity"
    );
}

// ---------------------------------------------------------------------------
// 6. An unclaimed pattern stays readable locally
// ---------------------------------------------------------------------------

/// A legacy pattern nobody ever claims is not lost by migration: its
/// `reusable_patterns` row is exactly as present after a full `--run` as
/// before, and `--status` lists it as retained with `owner_unclaimed` — never
/// silently dropped, and never silently attributed.
///
/// **Falsified by**: the local row disappearing, or `--status` failing to
/// name it with reason `owner_unclaimed`.
#[test]
fn an_unclaimed_pattern_stays_readable_locally_and_is_named_as_retained() {
    let f = fixture!();

    let _ = f.s.json(&["migrate", "--run"]);

    let still_local: i64 = f.s.query_column(&format!(
        "SELECT CAST(count(*) AS TEXT) FROM reusable_patterns WHERE id = '{}' AND deleted_at IS NULL",
        f.ids.pattern_unclaimed
    ))[0]
        .parse()
        .expect("a count");
    assert_eq!(
        still_local, 1,
        "an unclaimed legacy pattern must remain readable locally after migration runs"
    );

    let status = f.s.json(&["migrate", "--status"]);
    let retained = status["status"]["retained"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let row = retained
        .iter()
        .find(|r| r["reference"] == json!(format!("pattern:{}", f.ids.pattern_unclaimed)))
        .unwrap_or_else(|| {
            panic!("the unclaimed pattern never appears in --status's retained list: {retained:?}")
        });
    assert_eq!(row["reason"], json!("owner_unclaimed"));
}

// ---------------------------------------------------------------------------
// 7. Local pattern evidence never leaves
// ---------------------------------------------------------------------------

/// `pattern_applications` is machine-local evidence (FR-707) and never
/// drains: its row survives migration untouched, and the server has no
/// column anywhere that could carry it or the five other names the privacy
/// boundary refuses. The delivered pattern itself carries only the safe
/// shape — owner/domain/trust — nothing else.
///
/// **Falsified by**: the local evidence row disappearing or changing;
/// `shared_patterns` (the only table a pattern could land in) growing a
/// column named `signals`, `signal_digest`, `origin_ref`,
/// `sanitization_report`, `source_memory_id` or `origin_deleted`; the
/// delivered row's `owner_user_id`/`domain`/`trust` disagreeing with what the
/// claim and the safe shape require.
#[test]
fn local_pattern_evidence_never_leaves_and_a_delivered_row_carries_only_the_safe_shape() {
    let f = fixture!();

    // Schema-level, and independent of whether anything ever delivers: the
    // six privacy-boundary names have no home on the server at all.
    for column in [
        "signals",
        "signal_digest",
        "origin_ref",
        "sanitization_report",
        "source_memory_id",
        "origin_deleted",
    ] {
        let exists = f.server.count(&format!(
            "SELECT count(*) FROM information_schema.columns
              WHERE table_schema = 'public' AND table_name = 'shared_patterns'
                AND column_name = '{column}'"
        ));
        assert_eq!(
            exists, 0,
            "`shared_patterns` grew a column the privacy boundary refuses: {column}"
        );
    }

    let claim = f.s.json(&[
        "migrate",
        "--claim-patterns",
        &f.ids.pattern_claimable.to_string(),
    ]);
    let pattern_id = claim["claims"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["local_pattern_id"] == json!(f.ids.pattern_claimable.to_string()))
        .expect("the claimed pattern's own row")["pattern_id"]
        .as_str()
        .expect("a pattern id")
        .to_string();
    let _ = f.s.json(&["migrate", "--run"]);

    // Local evidence is untouched — migration does not read it to decide
    // anything, and never sends it.
    let evidence: i64 = f.s.query_column(&format!(
        "SELECT CAST(count(*) AS TEXT) FROM pattern_applications WHERE pattern_id = '{}'",
        f.ids.pattern_claimable
    ))[0]
        .parse()
        .expect("a count");
    assert_eq!(
        evidence, 1,
        "migration touched machine-local pattern evidence, which never drains (FR-707)"
    );

    // The delivered row carries only the safe shape.
    let rows = f.server.query_column(&format!(
        "SELECT owner_user_id || '|' || domain || '|' || trust
           FROM shared_patterns WHERE pattern_id = '{pattern_id}'"
    ));
    assert_eq!(rows.len(), 1, "the claimed pattern was never delivered");
    let parts: Vec<&str> = rows[0].split('|').collect();
    assert_eq!(
        parts[0],
        f.account.to_string(),
        "owner_user_id must be the claim's own owner"
    );
    assert_eq!(
        parts[1], "personal",
        "a promoted pattern is a personal-domain record"
    );
    assert_eq!(
        parts[2], "sanitized",
        "the server can only establish this one trust level"
    );
}
