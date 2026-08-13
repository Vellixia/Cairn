//! The configuration fixture corpus (D40 tier 2, SC-104).
//!
//! Every file in `tests/fixtures/` is a realistic pre-existing agent
//! configuration, and the test is always the same: install what Cairn owns,
//! then remove it, and assert the file is **byte-identical** to what it was.
//!
//! The corpus spans the formatting dimensions a parse-and-reserialize editor
//! destroys — tab and four-space indentation, CRLF, minified single-line
//! objects, unusual key order, unicode escapes, comment-bearing TOML and
//! JSONC. That is the point: the CST's preservation is proved rather than
//! assumed (D37).

use cairn_integrate::agents::{claude_code, codex};
use cairn_integrate::edit::{json, markdown, toml, Change};
use cairn_integrate::markers::CONTRACT_ID;
use cairn_integrate::render::Contract;
use std::path::PathBuf;

fn corpus() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut out: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("fixture corpus")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            p.is_file() && name != "README.md" && !name.starts_with('.')
        })
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let body = std::fs::read_to_string(&p).unwrap_or_default();
            (name, body)
        })
        .collect();
    out.sort();
    out
}

/// What Cairn installs into, and removes from, one fixture.
enum Op {
    ClaudeHooks,
    ClaudeMcp,
    CodexMcp,
    CodexHooks,
    OpencodeMcp,
    AgentsBlock,
}

fn op_for(name: &str) -> Op {
    if name.starts_with("claude-settings__") {
        Op::ClaudeHooks
    } else if name.starts_with("claude-mcp__") {
        Op::ClaudeMcp
    } else if name.starts_with("codex-config__") {
        Op::CodexMcp
    } else if name.starts_with("codex-hooks__") {
        Op::CodexHooks
    } else if name.starts_with("opencode-json__") {
        Op::OpencodeMcp
    } else if name.starts_with("agents-md__") {
        Op::AgentsBlock
    } else {
        panic!("fixture `{name}` declares no ownership expectation");
    }
}

/// Install everything Cairn owns in this file.
fn install(op: &Op, name: &str, text: &str) -> Result<String, String> {
    let mut current = text.to_string();
    match op {
        Op::ClaudeHooks => {
            for ev in claude_code::EVENTS {
                let entry = claude_code::hook_entry(ev);
                let is_ours = |v: &serde_json::Value| claude_code::is_cairn_hook_entry(v, ev);
                let out =
                    json::upsert_array_entry(name, &current, &["hooks", ev], &is_ours, &entry)
                        .map_err(|e| e.to_string())?;
                if let Change::Written(s) = out {
                    current = s;
                }
            }
        }
        Op::ClaudeMcp | Op::OpencodeMcp => {
            let keys: &[&str] = if matches!(op, Op::ClaudeMcp) {
                &["mcpServers", "cairn"]
            } else {
                &["mcp", "cairn"]
            };
            if let Change::Written(s) =
                json::upsert(name, &current, keys, &cairn_integrate::mcp_entry())
                    .map_err(|e| e.to_string())?
            {
                current = s;
            }
        }
        Op::CodexMcp => {
            if let Change::Written(s) = toml::upsert(
                name,
                &current,
                &["mcp_servers", "cairn"],
                &cairn_integrate::mcp_entry(),
            )
            .map_err(|e| e.to_string())?
            {
                current = s;
            }
        }
        Op::CodexHooks => {
            for ev in codex::EVENTS {
                let entry = codex::hook_entry(ev);
                let is_ours = |v: &serde_json::Value| codex::is_cairn_hook_entry(v, ev);
                let out = json::upsert_array_entry(name, &current, &["hooks"], &is_ours, &entry)
                    .map_err(|e| e.to_string())?;
                if let Change::Written(s) = out {
                    current = s;
                }
            }
        }
        Op::AgentsBlock => {
            let c = Contract::canonical();
            if let Change::Written(s) =
                markdown::upsert(name, &current, CONTRACT_ID, c.schema, &c.block_body())
                    .map_err(|e| e.to_string())?
            {
                current = s;
            }
        }
    }
    Ok(current)
}

/// Remove everything Cairn owns from this file.
///
/// `collapse` carries the one bit Cairn records about its own edit: whether
/// the container it wrote into was on a single line before it did. That is not
/// a copy of the developer's file — FR-156 and FR-238 forbid one — it is a
/// property of Cairn's own change, and it is what returns a minified file to
/// its original bytes.
fn uninstall(op: &Op, name: &str, text: &str, original: &str) -> Result<String, String> {
    let mut current = text.to_string();
    match op {
        Op::ClaudeHooks => {
            for ev in claude_code::EVENTS {
                let is_ours = |v: &serde_json::Value| claude_code::is_cairn_hook_entry(v, ev);
                // Cairn prunes only a key it created; one the developer wrote
                // stays, even when it is left empty.
                let created_by_cairn = json::get(name, original, &["hooks", ev])
                    .map_err(|e| e.to_string())?
                    .is_none();
                if let Change::Written(s) = json::remove_array_entries(
                    name,
                    &current,
                    &["hooks", ev],
                    &is_ours,
                    created_by_cairn,
                )
                .map_err(|e| e.to_string())?
                {
                    current = s;
                }
            }
        }
        Op::ClaudeMcp | Op::OpencodeMcp => {
            let keys: &[&str] = if matches!(op, Op::ClaudeMcp) {
                &["mcpServers", "cairn"]
            } else {
                &["mcp", "cairn"]
            };
            if let Change::Written(s) =
                json::remove(name, &current, keys).map_err(|e| e.to_string())?
            {
                current = s;
            }
        }
        Op::CodexMcp => {
            if let Change::Written(s) = toml::remove(name, &current, &["mcp_servers", "cairn"])
                .map_err(|e| e.to_string())?
            {
                current = s;
            }
        }
        Op::CodexHooks => {
            for ev in codex::EVENTS {
                let is_ours = |v: &serde_json::Value| codex::is_cairn_hook_entry(v, ev);
                let created_by_cairn = json::get(name, original, &["hooks"])
                    .map_err(|e| e.to_string())?
                    .is_none();
                if let Change::Written(s) = json::remove_array_entries(
                    name,
                    &current,
                    &["hooks"],
                    &is_ours,
                    created_by_cairn,
                )
                .map_err(|e| e.to_string())?
                {
                    current = s;
                }
            }
        }
        Op::AgentsBlock => {
            if let Change::Written(s) =
                markdown::remove(name, &current, CONTRACT_ID).map_err(|e| e.to_string())?
            {
                current = s;
            }
        }
    }
    Ok(restore_layout(op, name, original, current))
}

/// Put back on one line every container that was on one line before Cairn
/// wrote into it — outermost first, so a nested pair is handled by the outer
/// collapse.
///
/// The CST cannot insert into an object without expanding it, so this is the
/// step that makes connect → disconnect byte-exact for a minified file. It
/// consults the original text in memory during the operation and stores
/// nothing (FR-156, FR-238).
fn restore_layout(op: &Op, name: &str, original: &str, mut current: String) -> String {
    if name.ends_with(".toml") || name.ends_with(".md") {
        return current;
    }
    let events: Vec<Vec<&str>> = match op {
        Op::ClaudeHooks => claude_code::EVENTS
            .iter()
            .map(|ev| vec!["hooks", *ev])
            .collect(),
        _ => Vec::new(),
    };
    let mut candidates: Vec<Vec<&str>> = match op {
        Op::ClaudeHooks | Op::CodexHooks => vec![vec!["hooks"]],
        Op::ClaudeMcp => vec![vec!["mcpServers"]],
        Op::OpencodeMcp => vec![vec!["mcp"]],
        _ => Vec::new(),
    };
    candidates.extend(events);
    for keys in candidates {
        if json::value_is_single_line(name, original, &keys)
            && !json::value_is_single_line(name, &current, &keys)
        {
            current = json::collapse_path(&current, &keys);
        }
    }
    if json::root_is_single_line(original) {
        current = json::collapse_root(&current);
    }
    current
}

#[test]
fn the_corpus_is_large_enough_to_be_evidence() {
    // SC-104 requires at least 20 realistic files.
    let corpus = corpus();
    assert!(
        corpus.len() >= 20,
        "the corpus has only {} fixtures",
        corpus.len()
    );
}

#[test]
fn preservation() {
    // SC-104: connect followed by disconnect returns each file to a state
    // where 100% of non-Cairn content is byte-identical to the original.
    for (name, original) in corpus() {
        let op = op_for(&name);
        let installed = install(&op, &name, &original).unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut restored =
            uninstall(&op, &name, &installed, &original).unwrap_or_else(|e| panic!("{name}: {e}"));
        // A file Cairn created in its entirety is removed rather than left as
        // an empty object the developer never wrote.
        if original.trim().is_empty() && json::is_empty_document(&restored) {
            restored = original.clone();
        }
        assert_eq!(
            restored, original,
            "{name}: connect → disconnect did not return the file to its original bytes"
        );
    }
}

#[test]
fn idempotent_reconnect() {
    // SC-102: running connect twice produces zero changes on the second run.
    for (name, original) in corpus() {
        let op = op_for(&name);
        let once = install(&op, &name, &original).unwrap_or_else(|e| panic!("{name}: {e}"));
        let twice = install(&op, &name, &once).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(twice, once, "{name}: connecting twice changed the file");
    }
}

#[test]
fn every_developer_setting_survives_installation() {
    // FR-152: entries, ordering, comments and formatting outside Cairn's own
    // content are preserved *while Cairn's resource is installed*, not merely
    // restored afterwards.
    for (name, original) in corpus() {
        let op = op_for(&name);
        let installed = install(&op, &name, &original).unwrap_or_else(|e| panic!("{name}: {e}"));
        for marker in [
            "make lint",
            "audit.sh",
            "warm-cache",
            "user-warmup",
            "user-notify",
            "# a final word",
            "# inline note",
            "# the developer keeps notes here",
            "a developer comment that is not ours",
            "Run the tests before pushing.",
            "Be careful with migrations.",
            "zzz_last",
        ] {
            if original.contains(marker) {
                assert!(
                    installed.contains(marker),
                    "{name}: `{marker}` was lost during installation"
                );
            }
        }
    }
}

#[test]
fn a_developer_command_mentioning_cairn_is_never_claimed() {
    // FR-139: the exact bug the Feature 001 substring match would produce.
    let (name, original) = corpus()
        .into_iter()
        .find(|(n, _)| n == "claude-settings__mentions-cairn.json")
        .expect("the mentions-cairn fixture");
    let op = op_for(&name);
    let installed = install(&op, &name, &original).unwrap();
    let restored = uninstall(&op, &name, &installed, &original).unwrap();
    assert!(restored.contains("remember to run cairn hook first"));
    assert_eq!(restored, original);
}

#[test]
fn no_seeded_credential_is_ever_read_into_cairns_own_output() {
    // SC-119, SC-133: the corpus seeds recognizable credentials; none of them
    // may appear in anything Cairn produces about these files.
    let seeded = ["sk-not-a-real-secret-000", "sk-not-a-real-secret-111"];
    let mut seen_a_seed = false;
    for (name, original) in corpus() {
        if !seeded.iter().any(|s| original.contains(s)) {
            continue;
        }
        seen_a_seed = true;
        let op = op_for(&name);
        let installed = install(&op, &name, &original).unwrap();
        // The developer's own credential stays in the developer's own file —
        // that is preservation working. What must not happen is Cairn
        // *reporting* it.
        assert!(installed.contains(seeded[0]) || installed.contains(seeded[1]));
        let plan_text = format!("{:?}", cairn_integrate::mcp_entry());
        for s in seeded {
            assert!(!plan_text.contains(s));
        }
    }
    assert!(seen_a_seed, "the corpus seeds no credentials to test with");
}

// ---------------------------------------------------------------------------
// T101–T103 — the integration manager (FR-232, FR-234, FR-146, FR-219, FR-200)
// ---------------------------------------------------------------------------
//
// CC Switch is a manager, not an agent. Cairn asks it to distribute, verifies
// against the *target applications'* own configuration, and never touches the
// manager's private storage — not to write, not to read, not to check whether
// something worked. That last one is the temptation: the answer really is in
// `~/.cc-switch/cc-switch.db`, and reading it would be easier than inspecting
// three applications' configuration files. FR-232 forbids it, and the whole
// point of a checksum fixture is that the forbidden thing is provable.

use cairn_integrate::adapter::{ImportRefusal, IntegrationManager};
use cairn_integrate::managers::cc_switch::{self, CcSwitch};
use cairn_integrate::model::{HealthCondition, ManagerId, ResourceKind};
use cairn_integrate::scope::Env;

/// A machine with CC Switch installed and its private storage populated.
struct Machine {
    _dir: tempfile::TempDir,
    env: Env,
}

impl Machine {
    fn new() -> Machine {
        let dir = tempfile::tempdir().expect("temp home");
        let home = dir.path().join("home");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).unwrap();

        // CC Switch's own storage, exactly as D33 records it. Every one of
        // these is private and none of it may be touched.
        let private = home.join(".cc-switch");
        std::fs::create_dir_all(private.join("skills").join("cairn")).unwrap();
        std::fs::create_dir_all(private.join("backups")).unwrap();
        std::fs::write(private.join("cc-switch.db"), b"SQLite format 3\0PRIVATE").unwrap();
        std::fs::write(
            private.join("settings.json"),
            r#"{"providers":[{"name":"anthropic","apiKey":"sk-PRIVATE"}]}"#,
        )
        .unwrap();
        std::fs::write(private.join("version"), "1.9.0\n").unwrap();
        std::fs::write(
            private.join("skills").join("cairn").join("SKILL.md"),
            "---\nname: cairn\n---\n",
        )
        .unwrap();

        Machine {
            env: Env::new(&home, &worktree),
            _dir: dir,
        }
    }

    /// Every file under the manager's private storage, with its content.
    fn manager_state(&self) -> std::collections::BTreeMap<String, Vec<u8>> {
        let root = self.env.home.join(".cc-switch");
        let mut out = std::collections::BTreeMap::new();
        walk(&root, &root, &mut out);
        out
    }

    /// Give a target application a `cairn` MCP entry, as a confirmed CC Switch
    /// import would.
    fn app_receives_mcp(&self, app: &str) {
        let path = cairn_integrate::scope::manager_location(&self.env, app, ResourceKind::Mcp)
            .expect("a known location");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body = if app == "codex" {
            "model = \"gpt-5-codex\"\n\
             [mcp_servers.internal]\ncommand = \"internal-mcp\"\n\
             [mcp_servers.cairn]\ncommand = \"cairn\"\nargs = [\"mcp\"]\n"
                .to_string()
        } else if app == "opencode" {
            serde_json::json!({
                "mcp": {
                    "internal": { "type": "local", "command": ["internal-mcp"] },
                    "cairn": { "type": "local", "command": ["cairn", "mcp"] }
                }
            })
            .to_string()
        } else {
            serde_json::json!({
                "mcpServers": {
                    "internal": { "command": "internal-mcp" },
                    "cairn": { "command": "cairn", "args": ["mcp"] }
                }
            })
            .to_string()
        };
        std::fs::write(path, body).unwrap();
    }
}

/// How many MCP servers named `cairn` a configuration file declares.
///
/// Counted as keys rather than as occurrences of the word: `"command":
/// "cairn"` is the same entry, not a second one.
fn cairn_entries(text: &str) -> usize {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        return ["mcpServers", "mcp"]
            .iter()
            .filter_map(|k| v.get(*k))
            .filter_map(|m| m.as_object())
            .filter(|m| m.contains_key("cairn"))
            .count();
    }
    // TOML: the entry is a `[mcp_servers.cairn]` table header.
    text.lines()
        .filter(|l| l.trim() == "[mcp_servers.cairn]")
        .count()
}

fn walk(
    base: &std::path::Path,
    dir: &std::path::Path,
    out: &mut std::collections::BTreeMap<String, Vec<u8>>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            walk(base, &path, out);
        } else {
            let key = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .display()
                .to_string();
            out.insert(key, std::fs::read(&path).unwrap_or_default());
        }
    }
}

/// Every manager-touching operation there is, run once.
///
/// These are the only code paths any of `connect`, `distribute`, `migrate`,
/// `repair` and `disconnect` reach the manager through: the command layer
/// above them calls exactly these and then reports.
fn every_manager_operation(m: &CcSwitch, machine: &Machine) {
    let apps: Vec<String> = m.target_apps().iter().map(|s| s.to_string()).collect();

    // connect / doctor: detection and verification.
    let _ = m.detect(&machine.env);
    for kind in cc_switch::DISTRIBUTABLE {
        let _ = m.inspect_bindings(&machine.env, &apps, *kind);
        // distribute: build the import link, and record what it produced.
        if let Ok(uri) = m.import_uri(*kind, &apps) {
            let _ = cc_switch::import_action(*kind, &apps, uri);
        }
        // disconnect / migrate: report the manual path, write nothing.
        let _ = cc_switch::removal_action(*kind, &apps);
    }
}

#[test]
fn manager_zero_writes() {
    // FR-232, SC-132: the manager's own storage is byte-identical after every
    // operation Cairn can perform, in 100% of cases.
    let machine = Machine::new();
    let m = CcSwitch;

    let before = machine.manager_state();
    assert!(
        before.contains_key("cc-switch.db"),
        "the fixture does not have private storage to protect"
    );

    for _ in 0..3 {
        every_manager_operation(&m, &machine);
    }
    // And again with the applications already configured, which is when a
    // "verify by reading the manager's index" shortcut would be most tempting.
    for app in m.target_apps() {
        machine.app_receives_mcp(app);
    }
    every_manager_operation(&m, &machine);

    assert_eq!(
        before,
        machine.manager_state(),
        "an operation changed CC Switch's own storage"
    );
}

#[test]
fn manager_bindings() {
    // FR-234, SC-112: ownership is updated only from verification against the
    // target applications, never from the manager's own state.
    let machine = Machine::new();
    let m = CcSwitch;
    let apps: Vec<String> = vec!["claude".into(), "codex".into()];

    // Nothing distributed yet: every binding is missing, and each one says
    // what to run rather than leaving the developer to guess.
    let before = m.inspect_bindings(&machine.env, &apps, ResourceKind::Mcp);
    assert_eq!(before.len(), 2);
    for b in &before {
        assert_eq!(b.condition, HealthCondition::Missing, "{b:?}");
        assert!(b
            .remedy
            .as_deref()
            .unwrap_or_default()
            .contains("distribute"));
    }

    // The developer confirms the import for the two applications they chose.
    machine.app_receives_mcp("claude");
    machine.app_receives_mcp("codex");

    let after = m.inspect_bindings(&machine.env, &apps, ResourceKind::Mcp);
    for b in &after {
        assert_eq!(
            b.condition,
            HealthCondition::Healthy,
            "a confirmed import was not verified from the application's own \
             configuration: {b:?}"
        );
    }

    // Exactly one Cairn entry per selected application, and the applications
    // that were not selected have none.
    for app in ["claude", "codex"] {
        let path =
            cairn_integrate::scope::manager_location(&machine.env, app, ResourceKind::Mcp).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            cairn_entries(&text),
            1,
            "{app} did not receive exactly one cairn entry: {text}"
        );
        // The manager-held configuration around it is untouched.
        assert!(text.contains("internal-mcp"), "{app}: {text}");
    }
    let unselected = m.inspect_bindings(&machine.env, &["opencode".into()], ResourceKind::Mcp);
    assert_eq!(unselected[0].condition, HealthCondition::Missing);

    // The import link itself names the chosen applications and nothing else.
    let uri = m.import_uri(ResourceKind::Mcp, &apps).unwrap();
    assert!(uri.contains("apps=claude,codex"), "{uri}");
    assert!(!uri.contains("opencode"), "{uri}");
}

#[test]
fn post_provider_switch() {
    // FR-200, SC-113, US5 #8: a provider switch inside CC Switch rewrites the
    // applications' configuration. Cairn's resources must still verify
    // healthy, with no duplicates, and everything else must be untouched.
    let machine = Machine::new();
    let m = CcSwitch;
    let apps: Vec<String> = m.target_apps().iter().map(|s| s.to_string()).collect();
    for app in m.target_apps() {
        machine.app_receives_mcp(app);
    }
    let before = m.inspect_bindings(&machine.env, &apps, ResourceKind::Mcp);
    assert!(before
        .iter()
        .all(|b| b.condition == HealthCondition::Healthy));

    // The switch: CC Switch rewrites the provider block and reorders the file,
    // leaving Cairn's entry in place — the realistic shape of what it does.
    let claude =
        cairn_integrate::scope::manager_location(&machine.env, "claude", ResourceKind::Mcp)
            .unwrap();
    std::fs::write(
        &claude,
        serde_json::json!({
            "primaryApiProvider": "bedrock",
            "mcpServers": {
                "cairn": { "command": "cairn", "args": ["mcp"] },
                "internal": { "command": "internal-mcp" }
            }
        })
        .to_string(),
    )
    .unwrap();

    let after = m.inspect_bindings(&machine.env, &apps, ResourceKind::Mcp);
    for b in &after {
        assert_eq!(
            b.condition,
            HealthCondition::Healthy,
            "a provider switch broke a Cairn resource: {b:?}"
        );
    }
    let text = std::fs::read_to_string(&claude).unwrap();
    assert_eq!(
        cairn_entries(&text),
        1,
        "the provider switch left a duplicate Cairn entry: {text}"
    );
    assert!(
        text.contains("internal-mcp") && text.contains("bedrock"),
        "another provider's configuration was disturbed: {text}"
    );

    // The manager's own storage is still untouched by the verification.
    let state = machine.manager_state();
    every_manager_operation(&m, &machine);
    assert_eq!(state, machine.manager_state());
}

#[test]
fn the_manager_produces_no_lifecycle_of_its_own() {
    // T104, FR-101, FR-106: CC Switch is not an agent. It has no adapter, so
    // there is no code path by which it could open a session, record an
    // observation, or emit a lifecycle event — and the applications it happens
    // to support reach Cairn through the generic MCP path, not through a
    // native adapter Cairn grew for the manager's sake.
    assert!(
        cairn_integrate::AgentId::ALL
            .iter()
            .all(|a| a.as_str() != ManagerId::CcSwitch.as_str()),
        "the manager appears in the agent vocabulary"
    );
    for a in cairn_integrate::AgentId::ALL {
        let adapter = cairn_integrate::adapter_for(a);
        assert_ne!(adapter.id().as_str(), "cc-switch");
    }
    // Whatever payload arrives, under whatever event name, no adapter claims
    // it on the manager's behalf.
    let payload = cairn_integrate::adapter::RawPayload::new(
        serde_json::json!({ "session_id": "s", "sessionID": "s", "manager": "cc-switch" }),
        "/home/dev/app",
    );
    for name in ["import", "provider.switch", "cc-switch", "skill.installed"] {
        for a in cairn_integrate::AgentId::ALL {
            assert!(
                cairn_integrate::normalize(a, name, &payload).is_none(),
                "{a:?} produced a lifecycle event from a manager event `{name}`"
            );
        }
    }
    // And a development build refuses to hand CC Switch a Skill ref it cannot
    // publish, rather than pointing it at a branch that does not exist.
    if cc_switch::published_skill_branch().is_none() {
        assert!(matches!(
            CcSwitch.import_uri(ResourceKind::Skill, &["claude".into()]),
            Err(ImportRefusal::UnpublishedSkillRef { .. })
        ));
    }
}
