//! What a git remote actually says about a project, at every entry point that
//! screens on it (FR-546, FR-577, FR-822).
//!
//! # The claim
//!
//! `names_a_project` folds separators and asks whether the candidate text
//! *contains* an identity. That is the right test for a name, and it makes the
//! identity set load-bearing in both directions: every token in it refuses
//! every word that contains it, and every token missing from it is a
//! disclosure the screen will not catch.
//!
//! So the set has to be the project's identities and nothing else. A remote
//! contributes three: the host, the organisation and the repository. It does
//! not contribute the protocol, the SSH username, or a fragment of the host —
//! and the command side used to split on `.` with no filter at all, which put
//! the bare token `com` in the set for every project on `github.com` and
//! refused `compare`, `command`, `compile` and `component` as though they
//! named a project.
//!
//! # Why the tests drive real routes
//!
//! Calling `validate_global_content` with a hand-written identity list proves
//! the validator works and says nothing about what the routes hand it — which
//! is where the defect lived. Everything below goes through `/api/personal/
//! knowledge`, `/api/team/knowledge`, `/api/patterns` and `/api/sync/batch`,
//! so the identity derivation under test is the one production uses.
//!
//! # The two directions, always together
//!
//! Every fixture here proves acceptance *and* refusal for the same project.
//! Curing over-refusal by shrinking the identity set is the obvious wrong fix,
//! and only the refusal half notices it.

use cairn_e2e::feature005::Account;
use cairn_e2e::{post_json_status_bearer, Server};
use serde_json::{json, Value};
use uuid::Uuid;

macro_rules! server {
    () => {
        match Server::start_own_database() {
            Some(s) => s,
            None => {
                eprintln!("skipped: CAIRN_TEST_DATABASE_URL is not set");
                return;
            }
        }
    };
}

/// One account, one project, one remote — the smallest thing that has an
/// identity set at all.
struct Fixture {
    server: Server,
    owner: Account,
    project: Uuid,
    name: String,
}

fn fixture(server: Server, name: &str, remote: &str) -> Fixture {
    let (id, token) = server.new_user("identity-owner");
    let owner = Account {
        id,
        email: String::new(),
        token,
    };
    let project = Uuid::now_v7();
    server.execute(&format!(
        "INSERT INTO projects (id, name, repository_remote)
         VALUES ('{project}', '{name}', '{remote}')"
    ));
    server.execute(&format!(
        "INSERT INTO project_members (project_id, user_id) VALUES ('{project}', '{}')",
        owner.id
    ));
    Fixture {
        server,
        owner,
        project,
        name: name.to_string(),
    }
}

/// The GitHub SCP-style remote the defect was reported against.
fn acme(server: Server) -> Fixture {
    fixture(
        server,
        "Widgets Control Plane",
        "git@github.com:acme/widgets.git",
    )
}

// ---------------------------------------------------------------------------
// The four screened entry points, each reduced to "was this text refused, and
// as what"
// ---------------------------------------------------------------------------

/// `None` when the route accepted the text; `Some(class)` when it refused.
///
/// The class is what matters: a text refused as `command_shaped` says nothing
/// about project identity, and a test that only checked for failure would read
/// one as the other.
fn refusal_class(body: &Value, status: u16) -> Option<String> {
    if status == 200 {
        return None;
    }
    Some(class_of(
        body["error"]["message"].as_str().unwrap_or_default(),
    ))
}

/// The class out of a refusal message, whichever prefix the entry point wrote.
///
/// The command routes say `refused: <class>` and the synchronization path says
/// `refused by the content validator: <class>`. Both end in the class and
/// nothing else, so the tail after the last `": "` is it — and matching on the
/// tail rather than on either whole message keeps these tests indifferent to
/// wording they are not about.
fn class_of(message: &str) -> String {
    message
        .rsplit(": ")
        .next()
        .unwrap_or(message)
        .trim()
        .to_string()
}

fn personal(f: &Fixture, content: &str) -> Option<String> {
    let (body, status) = post_json_status_bearer(
        &f.server.base,
        "/api/personal/knowledge",
        &json!({
            "type": "convention",
            "content": content,
            "topic_key": "review.order",
            "value_key": format!("v{}", Uuid::now_v7().simple()),
        }),
        &f.owner.token,
    );
    refusal_class(&body, status)
}

fn team(f: &Fixture, content: &str) -> Option<String> {
    let (body, status) = post_json_status_bearer(
        &f.server.base,
        "/api/team/knowledge",
        &json!({
            "type": "convention",
            "content": content,
            "topic_key": "review.order",
            "value_key": format!("v{}", Uuid::now_v7().simple()),
        }),
        &f.owner.token,
    );
    refusal_class(&body, status)
}

/// The pattern route screens six fields; the text goes in `problem`, and the
/// rest is deliberately inert prose that no validator class matches.
fn pattern(f: &Fixture, content: &str) -> Option<String> {
    let (body, status) = post_json_status_bearer(
        &f.server.base,
        "/api/patterns",
        &json!({
            // Hyphenated, not `simple()`: a bare 32-character hex run is
            // exactly `encoded_secret_shape`, and the pattern route would
            // refuse every case here on the title before ever reaching the
            // identity screen.
            "title": format!("A note about something, number {}", Uuid::now_v7()),
            "problem": content,
            "root_cause": "A value is read before it is set.",
            "approach": "Set it first.",
        }),
        &f.owner.token,
    );
    refusal_class(&body, status)
}

/// The fifth entry point: the same screen, reached by pushing a record rather
/// than by calling a command route (`sync.rs`, D447).
fn sync_push(f: &Fixture, content: &str) -> Option<String> {
    let entity = Uuid::now_v7();
    let (body, status) = post_json_status_bearer(
        &f.server.base,
        "/api/sync/batch",
        &json!({
            "project_id": f.project,
            "items": [{
                "idempotency_key": format!("k-{entity}"),
                "entity_type": "personal_knowledge",
                "entity_id": entity,
                "operation": "upsert",
                "payload": {
                    "type": "convention",
                    "content": content,
                    "topic_key": "review.order",
                    "value_key": format!("v{}", entity.simple()),
                },
            }],
        }),
        &f.owner.token,
    );
    // **A batch answers 200 and reports each item.** One rejection does not
    // fail the call (`sync.rs::sync_batch`), so reading the HTTP status here
    // would record every refusal as an acceptance — which is how this helper
    // first reported the sync path as permissive when it was not.
    assert_eq!(status, 200, "the batch call itself failed: {body}");
    let item = &body["results"][0];
    if item["status"] != json!("rejected") {
        return None;
    }
    Some(class_of(
        item["error"]["message"].as_str().unwrap_or_default(),
    ))
}

/// One screened entry point: its name, and what it answers for a given text.
type Entry = (&'static str, fn(&Fixture, &str) -> Option<String>);

/// Every command entry point, so a repair proved at one of them cannot be
/// assumed at the others.
const COMMANDS: [Entry; 3] = [("personal", personal), ("team", team), ("pattern", pattern)];

/// Ordinary technical prose that names no project, chosen because each word
/// contains a fragment of `github.com`.
const PROSE: [&str; 4] = [
    "compare parser strategies",
    "command metadata is structured",
    "compile results are cached",
    "component boundaries are stable",
];

// ---------------------------------------------------------------------------
// 1. Ordinary prose is not a project name
// ---------------------------------------------------------------------------

/// Text whose only relationship to `git@github.com:acme/widgets.git` is that it
/// contains the letters of a TLD is accepted at every command entry point.
///
/// **Falsified by** splitting the remote on `.` again, or by emitting a bare
/// `com` from the host: `com` is a substring of all four phrases, and the
/// screen matches on containment.
#[test]
fn ordinary_prose_is_not_refused_as_naming_the_project() {
    let f = acme(server!());
    for (entry, call) in COMMANDS {
        for text in PROSE {
            assert_eq!(
                call(&f, text),
                None,
                "`{text}` was refused at the {entry} entry point, though it names \
                 nothing in `git@github.com:acme/widgets.git`"
            );
        }
    }
}

/// The same, at the synchronization entry point.
#[test]
fn ordinary_prose_is_not_refused_when_pushed_rather_than_commanded() {
    let f = acme(server!());
    for text in PROSE {
        assert_eq!(
            sync_push(&f, text),
            None,
            "`{text}` was refused on the sync path, though it names nothing in \
             `git@github.com:acme/widgets.git`"
        );
    }
}

// ---------------------------------------------------------------------------
// 2. The project's real identities are still refused
// ---------------------------------------------------------------------------

/// Host, organisation, repository and the project's own name are all still
/// refused as `project_identifying`, at every entry point.
///
/// This is the half that notices a repair which cured the over-refusal by
/// shrinking the identity set.
///
/// **Falsified by** dropping the host, the organisation or the repository from
/// the parser's output.
#[test]
fn the_projects_own_identities_are_still_refused_everywhere() {
    let f = acme(server!());
    let name = f.name.clone();
    let identities = ["github.com", "acme", "widgets", name.as_str()];
    for (entry, call) in COMMANDS {
        for identity in identities {
            let text = format!("the {identity} pipeline is slow");
            assert_eq!(
                call(&f, &text).as_deref(),
                Some("project_identifying"),
                "`{text}` was not refused as project-identifying at the {entry} \
                 entry point"
            );
        }
    }
    for identity in identities {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            sync_push(&f, &text).as_deref(),
            Some("project_identifying"),
            "`{text}` was not refused as project-identifying on the sync path"
        );
    }
}

/// The separator-folded spellings the validator's own contract says must
/// match: `github.com` names the project written as `githubcom` or
/// `github_com` too, because `fold_separators` drops everything that is not a
/// letter or a digit before comparing.
///
/// **Falsified by** a parser that emits the host only in one spelling and a
/// folding rule that stopped agreeing with it.
#[test]
fn separator_folded_spellings_of_an_identity_are_refused_too() {
    let f = acme(server!());
    for spelling in ["githubcom", "github_com", "github-com", "GitHub.com"] {
        let text = format!("the {spelling} pipeline is slow");
        assert_eq!(
            personal(&f, &text).as_deref(),
            Some("project_identifying"),
            "`{text}` was not refused, though folding makes it the host"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Remote forms agree
// ---------------------------------------------------------------------------

/// SCP-style and HTTPS spellings of one repository yield the same identities.
///
/// Asserted through the screen rather than by reading the parser's output, so
/// it is a claim about what the routes refuse rather than about an internal
/// representation.
///
/// **Falsified by** a parser that emits `https` for the URL form, or that
/// keeps the SSH username `git` for the SCP form: `git` and `https` are
/// substrings of ordinary prose, and the accepted half below would fail.
#[test]
fn scp_and_https_spellings_of_one_remote_screen_identically() {
    let mut server = Some(server!());
    let scp = acme(server.take().expect("a server"));
    let https = fixture(
        Server::start_own_database().expect("a second server"),
        "Widgets Control Plane",
        "https://github.com/acme/widgets.git",
    );

    for identity in ["github.com", "acme", "widgets"] {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            personal(&scp, &text).as_deref(),
            Some("project_identifying"),
            "SCP form did not refuse `{identity}`"
        );
        assert_eq!(
            personal(&https, &text).as_deref(),
            Some("project_identifying"),
            "HTTPS form did not refuse `{identity}`"
        );
    }
    for text in PROSE {
        assert_eq!(personal(&scp, text), None, "SCP form refused `{text}`");
        assert_eq!(personal(&https, text), None, "HTTPS form refused `{text}`");
    }
    // Neither spelling contributes its own syntax.
    for structural in ["https", "the git history is long"] {
        assert_eq!(
            personal(&scp, structural),
            None,
            "SCP form refused the structural word `{structural}`"
        );
        assert_eq!(
            personal(&https, structural),
            None,
            "HTTPS form refused the structural word `{structural}`"
        );
    }
}

/// A nested GitLab path contributes every group in it, and neither the scheme
/// nor the SSH username.
///
/// **Falsified by** a parser that keeps only the first path segment, which
/// would let `subgroup` name the project freely.
#[test]
fn a_nested_ssh_url_contributes_every_group_and_no_syntax() {
    let f = fixture(
        server!(),
        "Nested Group Project",
        "ssh://git@gitlab.com/group/subgroup/repo.git",
    );
    for identity in ["gitlab.com", "group", "subgroup", "repo"] {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            personal(&f, &text).as_deref(),
            Some("project_identifying"),
            "`{identity}` was not refused, though it is part of the remote"
        );
    }
    for structural in ["the ssh key rotated", "the git history is long"] {
        assert_eq!(
            personal(&f, structural),
            None,
            "`{structural}` was refused, though it names only URL syntax"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Position, not vocabulary
// ---------------------------------------------------------------------------

/// A path component keeps its identity even when its text is a structural
/// word.
///
/// `https://github.com/com/net.git` has an organisation literally named `com`
/// and a repository literally named `net`. Both name the project, and a
/// parser that filtered them by value — which is how the ingest side avoids
/// the TLD — would be a hole: content saying `com` would then name that
/// project undetected.
///
/// The cost is real and correct: for *this* project, prose containing `com`
/// genuinely is project-identifying, which is why the acceptance cases above
/// use a different fixture.
///
/// **Falsified by** filtering a path component because its value is `com`,
/// `org`, `net`, `git`, `ssh` or `www`.
#[test]
fn a_path_component_named_like_syntax_is_still_an_identity() {
    let f = fixture(
        server!(),
        "Structural Names",
        "https://github.com/com/net.git",
    );
    for identity in ["github.com", "com", "net"] {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            personal(&f, &text).as_deref(),
            Some("project_identifying"),
            "`{identity}` was not refused, though it is this remote's own \
             organisation, repository or host"
        );
    }
}

/// The same, for a repository whose name is `git` — the one word the parser
/// strips as a suffix and as an SSH username.
///
/// **Falsified by** trimming `.git` more than once, or by dropping a path
/// component equal to the SSH username.
#[test]
fn a_repository_actually_named_git_survives_suffix_trimming() {
    let f = fixture(server!(), "Literal Git", "git@github.com:acme/git.git");
    for identity in ["github.com", "acme", "git"] {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            personal(&f, &text).as_deref(),
            Some("project_identifying"),
            "`{identity}` was not refused, though it is part of this remote"
        );
    }
}

// ---------------------------------------------------------------------------
// 5. The two entry points tokenize the same remote the same way
// ---------------------------------------------------------------------------

/// The command side and the synchronization side derive the same remote
/// tokens.
///
/// What it asserts is that for one active membership, both paths agree about
/// what the remote says, in both directions. Which project *rows* each side
/// selects is the separate question §6 settles.
///
/// **Falsified by** either side keeping its own remote-splitting loop.
#[test]
fn the_command_and_sync_entry_points_tokenize_one_remote_identically() {
    let f = acme(server!());
    for identity in ["github.com", "acme", "widgets"] {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            personal(&f, &text).as_deref(),
            Some("project_identifying"),
            "the command path did not refuse `{identity}`"
        );
        assert_eq!(
            sync_push(&f, &text).as_deref(),
            Some("project_identifying"),
            "the sync path did not refuse `{identity}`"
        );
    }
    for text in PROSE {
        assert_eq!(
            personal(&f, text),
            None,
            "the command path refused `{text}`"
        );
        assert_eq!(sync_push(&f, text), None, "the sync path refused `{text}`");
    }
}

// ---------------------------------------------------------------------------
// 6. Which project rows each side screens against (FR-577a)
// ---------------------------------------------------------------------------

/// The deleted project's own name, which is an identity in its own right and
/// not only a token of its remote.
const DELETED_NAME: &str = "Kestrel Ledger";

/// Tokens contributed by each of [`memberships`]'s three remotes, by the
/// relationship the caller has to that project.
const LIVE_TOKENS: [&str; 2] = ["acme", "widgets"];
const DELETED_TOKENS: [&str; 2] = ["kestrelworks", "ledger"];
const REMOVED_TOKENS: [&str; 2] = ["northwind", "atlas"];

/// One account with three projects in three different relationships to it: a
/// live membership, a membership whose project has since been deleted, and a
/// membership that was removed. The returned fixture's project is the live one
/// — the only one a push may declare, because a push must name a project the
/// caller belongs to.
///
/// One fixture rather than three, because the claim is comparative: the same
/// caller, in one request each, must be refused for two of these projects and
/// accepted for the third. Three fixtures could each pass for the wrong reason
/// — an empty identity set accepts everything — and only a case that must be
/// *refused* beside it notices.
fn memberships(server: Server) -> Fixture {
    let (id, token) = server.new_user("membership-owner");
    let owner = Account {
        id,
        email: String::new(),
        token,
    };
    let live = Uuid::now_v7();
    let deleted = Uuid::now_v7();
    let removed = Uuid::now_v7();
    for (project, name, remote) in [
        (
            live,
            "Widgets Control Plane",
            "git@github.com:acme/widgets.git",
        ),
        (
            deleted,
            DELETED_NAME,
            "git@github.com:kestrelworks/ledger.git",
        ),
        (
            removed,
            "Northwind Atlas",
            "git@github.com:northwind/atlas.git",
        ),
    ] {
        server.execute(&format!(
            "INSERT INTO projects (id, name, repository_remote)
             VALUES ('{project}', '{name}', '{remote}')"
        ));
        server.execute(&format!(
            "INSERT INTO project_members (project_id, user_id) VALUES ('{project}', '{}')",
            owner.id
        ));
    }
    // A soft delete, which is the only kind there is: `sync.rs::tombstone`
    // stamps `deleted_at` and leaves `name` and `repository_remote` exactly
    // where they were. The tokens are therefore still there to screen with —
    // whether they still *do* is what this section settles.
    server.execute(&format!(
        "UPDATE projects SET deleted_at = now() WHERE id = '{deleted}'"
    ));
    // A membership removed the way an administrator removes one: the row goes,
    // the project stays.
    server.execute(&format!(
        "DELETE FROM project_members WHERE project_id = '{removed}' AND user_id = '{}'",
        owner.id
    ));
    Fixture {
        server,
        owner,
        project: live,
        name: "Widgets Control Plane".to_string(),
    }
}

/// A deleted project still contributes its identities, at every entry point.
///
/// Deletion is soft, and the knowledge outlives it: a personal or team record
/// promoted from a project is untouched by that project's deletion (FR-519),
/// and it keeps reaching every device its owner has. So the name is still
/// disclosable after the project has stopped existing, and the screen that
/// exists to refuse that disclosure has to still reach it (FR-577a, FR-822).
///
/// **Falsified by** either side of the screen filtering `deleted_at IS NULL`:
/// the command routes did not and the synchronization entry point did, and
/// whichever one narrows makes this project's name ordinary prose.
#[test]
fn a_deleted_project_still_contributes_its_identities() {
    let f = memberships(server!());
    let mut identities: Vec<String> = DELETED_TOKENS.iter().map(|t| t.to_string()).collect();
    identities.push(DELETED_NAME.to_string());
    for (entry, call) in COMMANDS {
        for identity in &identities {
            let text = format!("the {identity} pipeline is slow");
            assert_eq!(
                call(&f, &text).as_deref(),
                Some("project_identifying"),
                "`{text}` was accepted at the {entry} entry point, though it names \
                 a project this caller belongs to — deleted, and still named"
            );
        }
    }
    for identity in &identities {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            sync_push(&f, &text).as_deref(),
            Some("project_identifying"),
            "`{text}` was accepted on the sync path, though it names a deleted \
             project this caller belongs to"
        );
    }
}

/// A membership that was removed stops contributing identities — and the live
/// one beside it keeps contributing.
///
/// The union is over *memberships* (FR-577). A project the caller has no
/// standing in is not theirs to be caught naming, and a server that screened
/// against every project it stores would refuse most English and tell every
/// caller what every other team is called.
///
/// **Falsified by** a screen that stopped joining `project_members`, and — by
/// the second half — by an identity set that came back empty, which would make
/// the first half pass for a reason that has nothing to do with membership.
#[test]
fn a_removed_membership_stops_contributing_and_a_live_one_does_not() {
    let f = memberships(server!());
    for identity in REMOVED_TOKENS {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            personal(&f, &text),
            None,
            "`{text}` was refused, though this caller's membership of that \
             project was removed"
        );
        assert_eq!(
            sync_push(&f, &text),
            None,
            "`{text}` was refused on the sync path, though this caller's \
             membership of that project was removed"
        );
    }
    for identity in LIVE_TOKENS {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            personal(&f, &text).as_deref(),
            Some("project_identifying"),
            "`{text}` was accepted, so the identity set was not narrower — it \
             was empty, and every case above passed for that reason"
        );
        assert_eq!(
            sync_push(&f, &text).as_deref(),
            Some("project_identifying"),
            "`{text}` was accepted on the sync path, so that identity set was \
             empty too"
        );
    }
}

/// Every membership contributes at once, and the project a push *declares*
/// does not narrow what it is screened against.
///
/// This is the bypass the fifth entry point exists to close (D447): a client
/// naming project X while declaring it was working in project Y. The screen is
/// the union of the caller's memberships, so declaring the live project buys
/// nothing — including for the deleted project's tokens, which is exactly where
/// the two sides used to disagree.
///
/// **Falsified by** screening against the declared `project_id`, or by the
/// ingest side selecting a different set of project rows than the command side.
#[test]
fn the_declared_project_does_not_narrow_the_screen_at_ingest() {
    let f = memberships(server!());
    // Declared: the live project. Named: the deleted one, and the live one.
    for identity in LIVE_TOKENS.iter().chain(DELETED_TOKENS.iter()) {
        let text = format!("the {identity} pipeline is slow");
        assert_eq!(
            sync_push(&f, &text).as_deref(),
            Some("project_identifying"),
            "a push declaring the live project accepted `{text}`, which names a \
             project this caller belongs to"
        );
    }
    // A refused item is not persisted (FR-581), so the owner's record count is
    // the measure rather than the response alone.
    assert_eq!(
        f.server.count(&format!(
            "SELECT count(*) FROM personal_knowledge WHERE owner_user_id = '{}'",
            f.owner.id
        )),
        0,
        "a refused push left a record behind"
    );
    // And prose naming none of the three is still accepted, so this is not
    // passing because the ingest path refuses everything.
    for text in PROSE {
        assert_eq!(
            sync_push(&f, text),
            None,
            "the ingest path refused `{text}`, which names none of the three \
             projects"
        );
    }
}
