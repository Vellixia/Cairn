//! T120 — the evidence CI requires runs without an agent, a credential, or a
//! network (FR-204, FR-205, SC-124).
//!
//! This matters more than it looks. The most important tests in Feature 002
//! are about what happens when a real Claude Code, Codex or OpenCode is
//! installed — and if proving that required three authenticated vendor CLIs,
//! the tests would run on nobody's machine and in no pull request. D40's whole
//! structure exists to avoid that: adapters are proved against recorded
//! payloads and fixture filesystems, and the live half is release evidence a
//! person runs by hand.
//!
//! So the claim is: every adapter, the integration manager and the generic MCP
//! path are fully exercised by tests that need none of it. Asserted here
//! rather than assumed, by driving the whole surface in an environment with no
//! agent installed at all.

use cairn_e2e::Sandbox;
use serde_json::json;

/// Every command that touches an adapter, a manager, or the MCP path.
const SURFACE: [&[&str]; 7] = [
    &["agents"],
    &["doctor"],
    &["connect", "claude-code", "--dry-run"],
    &["connect", "codex", "--dry-run"],
    &["connect", "opencode", "--dry-run"],
    &["integration", "export", "mcp"],
    &["repair", "--dry-run"],
];

#[test]
fn the_whole_integration_surface_runs_with_no_agent_installed() {
    // No `install_agent`: nothing is detected, and nothing may crash, hang, or
    // demand a vendor binary.
    let s = Sandbox::new();
    s.must(&["init"]);

    for command in SURFACE {
        let out = s.cairn(command);
        assert!(
            out.code == 0 || out.code == 1,
            "`cairn {}` failed unexpectedly with exit {}: {}",
            command.join(" "),
            out.code,
            out.stderr
        );
        // Never a panic, and never a demand for something CI cannot provide.
        let text = format!("{}{}", out.stdout, out.stderr).to_lowercase();
        assert!(
            !text.contains("panicked"),
            "`cairn {}` panicked",
            command.join(" ")
        );
        for demand in ["api key", "log in", "authenticate", "not authenticated"] {
            assert!(
                !text.contains(demand),
                "`cairn {}` asked for {demand} in an environment that has none",
                command.join(" ")
            );
        }
    }
}

#[test]
fn every_adapter_is_exercised_without_its_vendor_being_present() {
    // The lifecycle path — the one thing that would most plausibly need a real
    // agent — is driven for all three adapters against a machine where none is
    // installed. Detection is filesystem-only by design (FR-105), so a
    // directory is all a fixture needs.
    let s = Sandbox::new();
    s.must(&["init"]);

    let events: [(&str, &str, serde_json::Value); 3] = [
        (
            "claude-code",
            "SessionStart",
            json!({ "session_id": "h-claude", "source": "startup" }),
        ),
        (
            "codex",
            "SessionStart",
            json!({ "session_id": "h-codex", "source": "startup" }),
        ),
        (
            "opencode",
            "session.created",
            json!({ "sessionID": "h-opencode" }),
        ),
    ];
    for (agent, event, payload) in events {
        let out = s.hook_as(agent, event, payload);
        assert_eq!(
            out.code, 0,
            "{agent}: the hook failed with no vendor present"
        );
    }
    s.settle_session_count(3);

    // And the sessions carry the provenance of the three adapters, so this is
    // an exercise of the adapters rather than of one code path three times.
    let agents: std::collections::BTreeSet<String> = s.json(&["session", "list"])["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|x| x["agent"].as_str().map(str::to_string))
        .collect();
    assert_eq!(agents.len(), 3, "{agents:?}");
}

#[test]
fn no_required_evidence_reaches_the_network() {
    // FR-105: detection and inspection make no network call. Asserted by
    // taking the network away: with no proxy reachable and no resolver
    // configured, every command still behaves.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);

    // Its own daemon, so the *daemon* is network-starved too and not only the
    // CLI that talks to it — and its endpoint comes from the helper, because
    // a socket is a filesystem path on Unix and a name in the `\\.\pipe\`
    // namespace on Windows. A path spelled by hand binds nothing there, and
    // the failure arrives as "cairnd did not start" rather than as anything
    // about sockets.
    let socket = cairn_e2e::sandbox_socket();

    for command in [
        vec!["agents"],
        vec!["connect", "claude-code", "--yes"],
        vec!["doctor"],
        vec!["disconnect", "claude-code"],
        // Leaves nothing behind: this daemon is not the sandbox's, so its
        // Drop does not stop it.
        vec!["daemon", "stop"],
    ] {
        let mut full = vec!["--json"];
        full.extend_from_slice(&command);
        let out = std::process::Command::new(cairn_e2e::binary("cairn"))
            .args(&full)
            .current_dir(s.repo_path())
            .env("CAIRN_HOME", s.cairn_home())
            .env("CAIRN_SOCKET", &socket)
            .env("CAIRND_BIN", cairn_e2e::binary("cairnd"))
            .env("HOME", s.fake_home())
            .env("XDG_CONFIG_HOME", s.fake_home().join(".config"))
            // Anything that tried to reach out would fail loudly here.
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "")
            .output()
            .expect("cairn runs");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let envelope: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        assert_eq!(
            envelope["ok"],
            true,
            "`cairn {}` failed with the network taken away: {text}{}",
            command.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn no_test_in_this_suite_requires_a_secret() {
    // FR-205: the live half of the evidence is release evidence a person runs
    // by hand, never a required check. A test that read a vendor token out of
    // the environment would quietly make it one.
    //
    // The check is on what the suite *reads from the environment*, not on
    // which words appear in it: several tests seed realistic credential shapes
    // into fixtures deliberately, and that is the opposite of a dependency on
    // one.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut reads: Vec<(String, String)> = Vec::new();
    let mut checked = 0usize;
    for dir in [root.join("tests"), root.join("src")] {
        for entry in std::fs::read_dir(&dir).expect("a test directory").flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            for line in body.lines() {
                let Some(rest) = line.split("env::var").nth(1) else {
                    continue;
                };
                let Some(open) = rest.find('"') else { continue };
                let Some(close) = rest[open + 1..].find('"') else {
                    continue;
                };
                reads.push((name.clone(), rest[open + 1..open + 1 + close].to_string()));
            }
            checked += 1;
        }
    }
    assert!(checked > 10, "only {checked} files were checked");

    // The one variable the suite reads, and the tests that read it skip
    // themselves when it is absent rather than failing — which is what keeps
    // macOS CI green without Docker.
    for (file, name) in &reads {
        assert_eq!(
            name, "CAIRN_TEST_DATABASE_URL",
            "{file} reads `{name}` from the environment, which required CI cannot provide"
        );
    }
    let sync = std::fs::read_to_string(root.join("tests/us6_sync.rs")).expect("us6_sync.rs");
    assert!(
        sync.contains("SKIPPED"),
        "the server suite does not skip itself"
    );
}

#[test]
fn an_event_cairn_declines_still_reads_what_the_agent_wrote() {
    // FR-193, FR-194: the hook is invisible when it does nothing. Exiting
    // without draining stdin is not invisible — the agent is writing the
    // payload at that moment and gets a broken pipe, which surfaces in *its*
    // logs as Cairn failing.
    let s = Sandbox::new();
    s.install_agent("claude-code");
    s.must(&["init"]);

    // A large payload, so the write cannot fit in the pipe buffer and has to
    // block on a reader actually being there.
    let filler = "x".repeat(256 * 1024);
    for event in ["UserPromptSubmit", "Notification", "PreToolUse", "Setup"] {
        let out = s.hook(
            event,
            json!({ "session_id": "declined", "prompt": filler, "message": filler }),
        );
        assert_eq!(
            out.code, 0,
            "the declined event `{event}` failed the agent: {}",
            out.stderr
        );
    }
}
