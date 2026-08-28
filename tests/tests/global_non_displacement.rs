//! Personal and team sections cannot displace project context (T154, T166,
//! T168–T171, T173; FR-473–FR-478, FR-480, FR-482, FR-506, FR-584, SC-418,
//! SC-419, SC-420, SC-451, SC-461, SC-462, SC-464).
//!
//! Two knowledge domains that are not scoped to the project were added to a
//! briefing that already fit in a fixed budget. Everything about whether that was
//! safe reduces to one question: can a personal note or a piece of team guidance
//! ever take space a project section would otherwise have had? The answer has to
//! be no under every budget, not the one budget a test happened to pick, which is
//! why the tests below sweep a matrix rather than asserting a single case.
//!
//! Driven against `cairn_core::context::assemble` directly. The budget arithmetic
//! lives there, the function is pure, and a test one layer up would be asserting
//! the daemon's plumbing as much as the invariant — while being slower and, worse,
//! able to pass because the plumbing happened to supply no candidates. Here the
//! candidates are supplied explicitly and are always present, which is exactly the
//! failure mode T168 exists to replace: a version of this test that ran against an
//! empty global store proved nothing at all.
//!
//! The one test that must be end to end is T171, because `depth` is a wire field
//! and the claim is that a real client can request it.

use cairn_core::context::{
    assemble, ContextInputs, Level0, PersonalCandidate, TeamCandidate, GLOBAL_SHARE_MAX,
};
use cairn_core::domain::{new_id, Importance, Project, RepositoryState};
use chrono::Utc;

/// The budget sweep every test runs over.
///
/// Deliberately spans the interesting boundaries rather than sampling randomly: a
/// budget too small for any global item at all, budgets where the 15% cap binds,
/// and budgets where the non-reserve pool binds first. Fixed rather than random,
/// because a failing case a rerun cannot reproduce is a failing case nobody fixes.
const BUDGETS: &[usize] = &[200, 500, 1000, 2000, 3000, 5000, 8000, 12000];

fn project() -> Project {
    Project {
        id: new_id(),
        name: "cairn".into(),
        git_common_dir: "/tmp/repo/.git".into(),
        repository_remote: None,
        linked: false,
        server_project_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    }
}

fn inputs<'a>(
    p: &'a Project,
    project_memory: &'a [String],
    personal: &'a [PersonalCandidate],
    team: &'a [TeamCandidate],
) -> ContextInputs<'a> {
    ContextInputs {
        project: p,
        repository: RepositoryState {
            branch: "main".into(),
            commit_sha: Some("abc1234".into()),
            staged: 1,
            unstaged: 2,
            untracked: 0,
        },
        task: None,
        previous_handoff: None,
        decisions: &[],
        known_failures: &[],
        task_memory: &[],
        branch_memory: &[],
        project_memory,
        patterns: &[],
        has_history: true,
        degraded: false,
        level0: Level0::default(),
        personal_notes: personal,
        team_guidance: team,
    }
}

/// Personal candidates that would rank if there were room.
///
/// Long enough to cost real tokens and phrased like the content this feature
/// exists to carry — not one-word placeholders that would fit in any leftover and
/// so never test the boundary.
fn personal_candidates(n: usize) -> Vec<PersonalCandidate> {
    (0..n)
        .map(|i| PersonalCandidate {
            id: new_id(),
            content: format!(
                "personal note {i}: prefer the workspace lockfile over a per-crate one, \
                 because a resolver that disagrees between them is the failure this avoids"
            ),
            importance: Importance::Normal,
        })
        .collect()
}

fn team_candidates(n: usize) -> Vec<TeamCandidate> {
    (0..n)
        .map(|i| TeamCandidate {
            id: new_id(),
            content: format!(
                "team guidance {i}: commit messages follow Conventional Commits, and a \
                 release tag is annotated so the changelog can be derived from history"
            ),
            importance: Importance::Normal,
        })
        .collect()
}

/// Enough project memory to fill any budget in the sweep.
fn project_memory(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| {
            format!(
                "project memory {i}: the retry backoff is exponential with a thirty second \
                 ceiling, and the estimator undercounts multi-byte content by about four tokens"
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// T168 / FR-475 / SC-418 — a full budget leaves nothing for either domain
// ---------------------------------------------------------------------------

/// With the project's own sections filling the budget, the briefing is
/// byte-identical to the project-only baseline — with real, highly-rankable
/// global candidates present the whole time.
///
/// The candidates being present is the point. An earlier shape of this test could
/// pass against an empty global store, which proves that assembly does not invent
/// content, not that it declines to displace anything.
///
/// Falsified by letting `admit_global` draw on `general_remaining()` instead of
/// `remaining_non_reserve()`, or by moving either section ahead of any project
/// section.
#[test]
fn a_full_project_budget_leaves_the_briefing_byte_identical() {
    let p = project();
    // Enough to exhaust even the largest budget in the sweep. Too little and
    // the premise fails silently at the top end: there is headroom, global
    // content is legitimately admitted, and the test reports a displacement that
    // never happened.
    let memory = project_memory(2000);
    let personal = personal_candidates(8);
    let team = team_candidates(8);

    for &budget in BUDGETS {
        let baseline = assemble(&inputs(&p, &memory, &[], &[]), budget);
        let with_global = assemble(&inputs(&p, &memory, &personal, &team), budget);

        assert_eq!(
            serde_json::to_string(&baseline.briefing).unwrap(),
            serde_json::to_string(&with_global.briefing).unwrap(),
            "at budget {budget} the global sections changed a briefing whose project \
             sections had already consumed the pool"
        );
        assert_eq!(
            baseline.estimated_tokens, with_global.estimated_tokens,
            "at budget {budget} the global sections were charged for something"
        );
    }
}

// ---------------------------------------------------------------------------
// T169 / FR-474 / FR-480 / FR-584 / SC-419 — with headroom, bounded and last
// ---------------------------------------------------------------------------

/// With headroom: both sections appear, bounded by
/// `min(floor(total * 0.15), remaining_non_reserve)`, and `estimated_tokens`
/// never exceeds the budget.
///
/// Falsified by removing either term of the cap, or by charging global content to
/// the reserve.
#[test]
fn with_headroom_the_global_sections_appear_bounded_and_within_budget() {
    let p = project();
    let personal = personal_candidates(20);
    let team = team_candidates(20);
    // One short project memory, so the pool is mostly unspent.
    let memory = vec!["a single short project memory".to_string()];

    let mut ever_admitted = false;
    for &budget in BUDGETS {
        let baseline = assemble(&inputs(&p, &memory, &[], &[]), budget);
        let out = assemble(&inputs(&p, &memory, &personal, &team), budget);

        assert!(
            out.estimated_tokens <= budget,
            "at budget {budget} the assembled context exceeded it: {}",
            out.estimated_tokens
        );

        let global_spend = out
            .estimated_tokens
            .saturating_sub(baseline.estimated_tokens);
        let cap = (budget as f64 * GLOBAL_SHARE_MAX).floor() as usize;
        assert!(
            global_spend <= cap,
            "at budget {budget} the global sections spent {global_spend}, over the \
             documented cap of {cap}"
        );

        let admitted =
            !out.briefing.personal_notes.is_empty() || !out.briefing.team_guidance.is_empty();
        ever_admitted |= admitted;

        // Ordering: the two sections are the last two the briefing carries, and
        // personal precedes team. Asserted on the rendered order rather than on
        // an index, because the rendered order is what an agent reads.
        if admitted {
            let rendered = serde_json::to_value(&out.briefing).unwrap();
            let keys: Vec<String> = rendered
                .as_object()
                .expect("briefing object")
                .keys()
                .cloned()
                .collect();
            let personal_at = keys.iter().position(|k| k == "personal_notes");
            let team_at = keys.iter().position(|k| k == "team_guidance");
            if let (Some(pi), Some(ti)) = (personal_at, team_at) {
                assert!(
                    pi < ti,
                    "team guidance is serialized ahead of personal notes at budget {budget}"
                );
            }
        }
    }

    assert!(
        ever_admitted,
        "no budget in the sweep admitted any global content, so this test asserted \
         a bound nothing ever reached"
    );
}

// ---------------------------------------------------------------------------
// T170 / FR-584 / SC-451 — released reserve is not the global pool
// ---------------------------------------------------------------------------

/// A large, mostly unspent Level 0 reserve is released to the general pool, and
/// the global sections consume none of it.
///
/// This is the defect D449 exists to prevent, and it is invisible to a test that
/// only checks `estimated_tokens <= budget`: `general_remaining()` includes the
/// returned reserve and `remaining_non_reserve()` does not, so an implementation
/// reading the first stays inside the budget while spending reserve that Level 0
/// gave back for project content, not for this.
///
/// Falsified by substituting `general_remaining()` for `remaining_non_reserve()`
/// in `admit_global`.
#[test]
fn global_sections_never_spend_released_reserve() {
    let p = project();
    let personal = personal_candidates(20);
    let team = team_candidates(20);
    let memory = vec!["a single short project memory".to_string()];

    for &budget in BUDGETS {
        // A deliberately large reserve fraction, with nothing in Level 0 to
        // spend it: every reserved token is released to the general pool.
        let mut level0 = Level0::default();
        level0.caps.reserve_fraction = 0.9;

        let mut with_reserve = inputs(&p, &memory, &personal, &team);
        with_reserve.level0 = level0;
        let out = assemble(&with_reserve, budget);

        let mut baseline_inputs = inputs(&p, &memory, &[], &[]);
        let mut baseline_level0 = Level0::default();
        baseline_level0.caps.reserve_fraction = 0.9;
        baseline_inputs.level0 = baseline_level0;
        let baseline = assemble(&baseline_inputs, budget);

        let global_spend = out
            .estimated_tokens
            .saturating_sub(baseline.estimated_tokens);
        // The non-reserve pool is the 10% that was never reserved. Global spend
        // must fit inside it, never inside the 90% that came back.
        let non_reserve = budget - (budget as f64 * 0.9).floor() as usize;
        assert!(
            global_spend <= non_reserve,
            "at budget {budget} the global sections spent {global_spend} against a \
             non-reserve pool of {non_reserve}; they reached into released reserve"
        );
    }
}

// ---------------------------------------------------------------------------
// T154 / FR-476 / SC-462 — personal wins a tie with team
// ---------------------------------------------------------------------------

/// Where one personal and one team item compete for the same remaining space and
/// only one fits, the personal item is the one included — across the sweep.
///
/// Personal ahead of team is a deliberate ordering, not an implementation detail:
/// a note the user wrote themselves is more likely to be what they meant than a
/// server-wide default they may never have read.
///
/// Falsified by swapping the two `admit_global_section` calls.
#[test]
fn where_only_one_fits_the_personal_item_is_the_one_included() {
    let p = project();
    let memory = vec!["a single short project memory".to_string()];
    // One of each, identical in length so neither wins on size.
    let personal = vec![PersonalCandidate {
        id: new_id(),
        content: "P".repeat(400),
        importance: Importance::Normal,
    }];
    let team = vec![TeamCandidate {
        id: new_id(),
        content: "T".repeat(400),
        importance: Importance::Normal,
    }];

    let mut saw_a_tie = false;
    for &budget in BUDGETS {
        let out = assemble(&inputs(&p, &memory, &personal, &team), budget);
        let got_personal = !out.briefing.personal_notes.is_empty();
        let got_team = !out.briefing.team_guidance.is_empty();

        if got_personal && !got_team {
            saw_a_tie = true;
        }
        assert!(
            !(got_team && !got_personal),
            "at budget {budget} the team item was admitted while the personal one, of \
             identical cost, was not"
        );
    }

    assert!(
        saw_a_tie,
        "no budget in the sweep produced the case where exactly one of the two fits, \
         so this test never exercised the precedence it is about"
    );
}

// ---------------------------------------------------------------------------
// T166 / FR-482 / SC-464 — an importance hint changes nothing
// ---------------------------------------------------------------------------

/// An importance hint of **every** supported value leaves the assembled context
/// byte-identical.
///
/// The variant list is read from the enum rather than written out here, so a new
/// variant fails this test instead of quietly going unexercised. Importance ranks
/// within a bucket and does nothing else (FR-482) — and it must not be a back door
/// into reserved context, which is what a hint that changed section precedence
/// would become.
///
/// Falsified by reading `importance` anywhere in `admit_global`.
#[test]
fn an_importance_hint_on_a_global_item_changes_nothing_at_all() {
    let p = project();
    let memory = vec!["a single short project memory".to_string()];
    let all = [Importance::Low, Importance::Normal, Importance::High];

    // The enum's own list, so a fourth variant is a compile error here rather
    // than an untested value.
    fn exhaustive(i: Importance) -> &'static str {
        match i {
            Importance::Low => "low",
            Importance::Normal => "normal",
            Importance::High => "high",
        }
    }
    assert_eq!(all.map(exhaustive).len(), 3);

    for &budget in BUDGETS {
        let mut rendered: Option<String> = None;
        for importance in all {
            let personal: Vec<PersonalCandidate> = personal_candidates(6)
                .into_iter()
                .map(|mut c| {
                    c.importance = importance;
                    c
                })
                .collect();
            let team: Vec<TeamCandidate> = team_candidates(6)
                .into_iter()
                .map(|mut c| {
                    c.importance = importance;
                    c
                })
                .collect();
            let out = assemble(&inputs(&p, &memory, &personal, &team), budget);
            let text = serde_json::to_string(&out.briefing).unwrap();
            match &rendered {
                None => rendered = Some(text),
                Some(first) => assert_eq!(
                    first,
                    &text,
                    "at budget {budget} the importance hint `{}` changed the briefing",
                    exhaustive(importance)
                ),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// T171 / SC-420 — `depth: "minimum"` excludes both domains
// ---------------------------------------------------------------------------

/// A context request at `depth: "minimum"` contains zero personal or team
/// content regardless of available budget, and observably differs from standard
/// depth.
///
/// End to end, because the claim is that a real client can ask for it: `depth`
/// travelled on the wire type for a whole feature without any client being able
/// to set it, which is indistinguishable from the field not existing.
///
/// Falsified by making `depth` configurable-overridable, or by unwiring it from
/// either the tool surface or the daemon.
#[test]
fn a_minimum_depth_request_carries_no_global_content() {
    let s = cairn_e2e::Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    let cwd = s.repo_path().to_string_lossy().to_string();

    // Something in each domain that would otherwise be admitted, and a project
    // memory so the briefing is not empty either way.
    s.must(&[
        "memory",
        "add",
        "--type",
        "convention",
        "--scope",
        "project",
        "the retry backoff is exponential",
    ]);
    let created = mcp.tool_result(
        "cairn_remember",
        serde_json::json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "prefer the workspace lockfile over a per-crate one",
        }),
        &cwd,
    );
    assert_eq!(
        created["isError"], false,
        "personal create failed: {created}"
    );

    // `cairn_context` renders markdown with a JSON block appended, so the
    // assertion is on the rendered form — which is also what an agent actually
    // reads, and therefore the honest place to assert a section's absence.
    let standard = mcp.tool("cairn_context", serde_json::json!({}), &cwd);
    assert!(
        standard.contains("## Personal notes"),
        "standard depth carried no personal notes, so the minimum-depth comparison \
         below would pass against a briefing that never had any:\n{standard}"
    );

    let minimum = mcp.tool(
        "cairn_context",
        serde_json::json!({ "depth": "minimum" }),
        &cwd,
    );
    assert!(
        !minimum.contains("## Personal notes"),
        "minimum depth carried personal notes:\n{minimum}"
    );
    assert!(
        !minimum.contains("## Team guidance"),
        "minimum depth carried team guidance:\n{minimum}"
    );
    assert!(
        !minimum.contains("\"personal_notes\""),
        "minimum depth carried a personal_notes field in its payload:\n{minimum}"
    );
    assert_ne!(
        standard, minimum,
        "minimum depth produced the same briefing as standard, so the field is not \
         reaching the daemon at all"
    );
}

// ---------------------------------------------------------------------------
// T173 / FR-506 / SC-461 — nothing creates global content implicitly
// ---------------------------------------------------------------------------

/// Across a workload of recall, search, context assembly and synchronization with
/// **no** promotion or creation request issued, the personal and team record
/// counts do not change.
///
/// Reading must never write. The risk is not malice but convenience: a recall path
/// that "helpfully" promoted a frequently-matched memory, or a context assembler
/// that cached a rendered note as a personal record, would each be a plausible
/// optimisation and each would silently populate a domain the user never chose to
/// put anything in.
///
/// Falsified by any write on a read path.
#[test]
fn no_read_path_creates_global_content() {
    let s = cairn_e2e::Sandbox::new();
    let mut mcp = cairn_e2e::Mcp::start(&s);
    let cwd = s.repo_path().to_string_lossy().to_string();

    for i in 0..5 {
        s.must(&[
            "memory",
            "add",
            "--type",
            "convention",
            "--scope",
            "project",
            &format!("a project convention number {i}"),
        ]);
    }
    // One deliberate personal record, so the counts are non-zero and a test that
    // compared 0 to 0 could not pass by accident.
    let created = mcp.tool_result(
        "cairn_remember",
        serde_json::json!({
            "action": "create",
            "domain": "personal",
            "type": "fact",
            "content": "one deliberately created personal note",
        }),
        &cwd,
    );
    assert_eq!(
        created["isError"], false,
        "personal create failed: {created}"
    );

    let before = counts(&s);
    assert_eq!(before, ("1".to_string(), "0".to_string()));

    // The workload: every read surface, several times, plus a synchronization
    // attempt. None of these is a creation request.
    for _ in 0..3 {
        mcp.tool_result("cairn_context", serde_json::json!({}), &cwd);
        mcp.tool_result(
            "cairn_context",
            serde_json::json!({ "depth": "minimum" }),
            &cwd,
        );
        mcp.tool_result(
            "cairn_search",
            serde_json::json!({ "query": "convention" }),
            &cwd,
        );
        mcp.tool_result(
            "cairn_search",
            serde_json::json!({ "query": "note", "domains": ["personal", "team"] }),
            &cwd,
        );
        s.cairn(&["personal", "list"]);
        s.cairn(&["team", "list"]);
        s.cairn(&["status"]);
        s.cairn(&["sync", "status"]);
        s.cairn(&["sync", "now"]);
    }

    assert_eq!(
        counts(&s),
        before,
        "a read path created global content: {before:?} became {:?}",
        counts(&s)
    );
}

fn counts(s: &cairn_e2e::Sandbox) -> (String, String) {
    let personal = s
        .query_column("SELECT CAST(COUNT(*) AS TEXT) FROM personal_knowledge")
        .first()
        .cloned()
        .unwrap_or_default();
    let team = s
        .query_column("SELECT CAST(COUNT(*) AS TEXT) FROM team_knowledge")
        .first()
        .cloned()
        .unwrap_or_default();
    (personal, team)
}
