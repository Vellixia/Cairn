//! Which agents receive a canonical pattern automatically, and at which
//! delivery points (FR-838a, FR-838b, FR-895a).
//!
//! # Why this is separate from the other pattern tests
//!
//! The delivery and isolation suites prove *what* is rendered and *whose* it
//! is. This proves *who gets it and when* — the per-agent half of the claim,
//! which is the half a vendor change breaks quietly. Claude Code and Codex CLI
//! both deliver at session open and at prompt time; OpenCode declines both, and
//! the decline is Cairn's decision rather than a vendor absence.

use cairn_e2e::{attach_server, post_json_status_bearer, Sandbox, Server};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use uuid::Uuid;

const SETTLE: Duration = Duration::from_secs(30);

struct Linked {
    sandbox: Sandbox,
    #[allow(dead_code)]
    project: Uuid,
    account: Uuid,
    pattern_title: String,
    pattern_approach: String,
}

/// A linked project whose account owns exactly one canonical pattern.
fn linked(server: &Server, label: &str) -> Linked {
    let sandbox = Sandbox::new();
    let remote = format!(
        "git@localhost:cairnfixture/{label}-{}.git",
        Uuid::now_v7().simple()
    );
    sandbox.git(&["remote", "add", "origin", &remote]);
    sandbox.must(&["init"]);
    let (account, token) = server.new_user(label);
    let (created, status) = post_json_status_bearer(
        &server.base,
        "/api/projects",
        &json!({ "name": label, "repository_remote": remote }),
        &token,
    );
    assert_eq!(status, 200, "create project: {created}");
    let project: Uuid = created["id"].as_str().expect("id").parse().expect("uuid");
    attach_server(&sandbox, server, &token);
    sandbox.must(&["link", "--project", &project.to_string()]);

    // Canonical, on the server, owned by this account. Distinctive strings so
    // an assertion cannot pass on somebody else's pattern.
    let marker = Uuid::now_v7().simple().to_string();
    let title = format!("canonical-{marker}-title");
    let approach = format!("canonical-{marker}-approach");
    server.execute(&format!(
        "INSERT INTO shared_patterns
             (pattern_id, owner_user_id, domain, title, problem, root_cause, approach,
              content_key)
         VALUES ('{}', '{account}', 'personal', '{title}', 'p-{marker}', 'rc-{marker}',
                 '{approach}', 'ck-{marker}')",
        Uuid::now_v7()
    ));
    Linked {
        sandbox,
        project,
        account,
        pattern_title: title,
        pattern_approach: approach,
    }
}

/// One more canonical pattern for the same account, after the fact.
fn another_pattern(server: &Server, account: Uuid) -> (String, String) {
    let marker = Uuid::now_v7().simple().to_string();
    let title = format!("canonical-{marker}-title");
    let approach = format!("canonical-{marker}-approach");
    server.execute(&format!(
        "INSERT INTO shared_patterns
             (pattern_id, owner_user_id, domain, title, problem, root_cause, approach,
              content_key)
         VALUES ('{}', '{account}', 'personal', '{title}', 'p-{marker}', 'rc-{marker}',
                 '{approach}', 'ck-{marker}')",
        Uuid::now_v7()
    ));
    (title, approach)
}

/// The context an agent actually received, or the empty string when the vendor
/// has no context surface.
fn context_from(out: &cairn_e2e::CliResult) -> String {
    serde_json::from_str::<Value>(out.stdout.trim())
        .ok()
        .and_then(|v| {
            v["hookSpecificOutput"]["additionalContext"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn settle(what: &str, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + SETTLE;
    while Instant::now() < deadline {
        if predicate() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for: {what}");
}

/// One agent's session-open and prompt-time delivery, asserted on the text the
/// agent received.
fn both_delivery_points(agent: &str, key_field: &str) {
    let Some(server) = Server::start_own_database() else {
        eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
        return;
    };
    let l = linked(&server, agent);
    let key = format!("{agent}-{}", Uuid::now_v7());

    let open = l.sandbox.hook_as(
        agent,
        "SessionStart",
        json!({ key_field: key, "source": "startup" }),
    );
    assert!(open.ok(), "{agent} SessionStart: {}", open.stderr);
    let at_open = context_from(&open);
    assert!(
        at_open.contains(&l.pattern_title) && at_open.contains(&l.pattern_approach),
        "{agent} received no canonical pattern at session open. The server \
         selected it, budgeted it and traced it; if the agent did not read it, \
         the reference records a delivery that did not happen:\n{at_open}"
    );

    // The session has to reach the server before a prompt-time retrieval can
    // bind to it.
    settle("the session reaches the server", || {
        server.count("SELECT count(*) FROM sessions") > 0
    });

    // **A second canonical pattern, created after the session opened.**
    //
    // Prompt-time delivery is deliberately not a repeat of session open — the
    // server excludes what this session has already been given, which is the
    // rule that stops the same briefing arriving twice. So asserting the
    // *first* pattern again at prompt time would be asserting the opposite of
    // the contract. What must hold is that a canonical pattern the session has
    // not yet seen reaches the agent at this delivery point too.
    let (later_title, later_approach) = another_pattern(&server, l.account);

    let prompt = l.sandbox.hook_as(
        agent,
        "UserPromptSubmit",
        json!({ key_field: key, "prompt": "what do we know about this area" }),
    );
    assert!(prompt.ok(), "{agent} UserPromptSubmit: {}", prompt.stderr);
    let at_prompt = context_from(&prompt);
    assert!(
        at_prompt.contains(&later_title) && at_prompt.contains(&later_approach),
        "{agent} received no canonical pattern at prompt time. Both delivery \
         points are automatic for this vendor (FR-838a), and a briefing that \
         drops the pattern at one of them is a reference delivered at \
         neither:\n{at_prompt}"
    );
    assert!(
        !at_prompt.contains(&l.pattern_title),
        "{agent}'s prompt-time briefing restated the pattern its session-open \
         briefing already delivered, which is the one thing prompt-time \
         delivery must not do:\n{at_prompt}"
    );
}

/// Claude Code: canonical patterns at session open and at prompt time.
///
/// **Falsified by** dropping the server's pattern section at either delivery
/// point.
#[test]
fn claude_code_receives_the_canonical_pattern_at_both_delivery_points() {
    both_delivery_points("claude-code", "session_id");
}

/// Codex CLI: the same, at both points.
///
/// The README used to say Codex delivered at session open only. It routes
/// `UserPromptSubmit` exactly as Claude Code does, and this is what says so.
#[test]
fn codex_receives_the_canonical_pattern_at_both_delivery_points() {
    both_delivery_points("codex", "thread_id");
}

/// OpenCode's automatic delivery stays declined, and the decline stays Cairn's
/// (FR-838b).
///
/// OpenCode 2 *does* expose prompt and context hooks — they are beta, and Cairn
/// declines to rest an automatic guarantee on them. Reporting that as a vendor
/// absence would blame OpenCode for a choice Cairn made, so the matrix says
/// `declined_by_cairn` and this asserts it has not drifted.
///
/// **Falsified by** OpenCode acquiring automatic delivery, or by the decline
/// being relabelled as a vendor absence.
#[test]
fn opencode_still_declines_automatic_delivery_and_says_whose_decision_it_is() {
    use cairn_integrate::capability::{declared_matrix, MatrixCapability, MatrixStatus};

    let declared = declared_matrix("opencode");
    for wanted in [
        MatrixCapability::DeliverSessionOpen,
        MatrixCapability::DeliverPromptTime,
        MatrixCapability::DeliverPostCompaction,
    ] {
        let key = wanted.key();
        let cell = declared
            .iter()
            .find(|c| c.capability == key)
            .unwrap_or_else(|| panic!("{key} is missing from OpenCode's matrix"));
        assert_eq!(
            cell.status,
            MatrixStatus::DeclinedByCairn,
            "{key} reads `{}`. OpenCode exposes these hooks; declining to rest \
             an automatic guarantee on a beta surface is Cairn's decision, and \
             recording it as a vendor absence blames the vendor for it",
            cell.status.as_str()
        );
    }

    // The matrix is the boundary here, and the hook's stdout is not.
    //
    // Cairn does write a briefing to stdout for OpenCode, and that has always
    // been true: `emit_context` says so, and says why it reaches nobody —
    // OpenCode's installed plugin spawns `cairn hook` with stdout ignored. So
    // an assertion about what Cairn *writes* would be testing a pipe that is
    // not connected, and would fail on behaviour this repair is required to
    // leave alone. What the decline actually consists of is the declared
    // matrix above: no automatic delivery is claimed for this vendor, and the
    // reason recorded is Cairn's own decision.
    let receipt = declared
        .iter()
        .find(|c| c.capability == MatrixCapability::Receipt.key())
        .expect("the receipt cell");
    assert_ne!(
        receipt.status,
        MatrixStatus::Supported,
        "OpenCode reports a delivery receipt for deliveries it declines to \
         receive, which would be evidence of something that did not happen"
    );
}
