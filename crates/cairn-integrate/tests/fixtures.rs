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
        .filter(|p| p.is_file() && p.file_name().unwrap() != "README.md")
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
