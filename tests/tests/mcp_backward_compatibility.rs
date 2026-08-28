//! T127 — the recorded Feature 001/002 corpus, replayed against this build
//! (FR-495, FR-497, SC-323).
//!
//! `tests/knowledge/baseline/mcp_calls.json` was captured **before** migration
//! 0005 existed. Replaying it here and comparing every pre-existing field is
//! what makes "a call carrying only Feature 001 arguments behaves exactly as it
//! does today, plus new read-only fields" a gate rather than an intention.
//!
//! The comparison is deliberately one-directional: every field the baseline
//! recorded must still be present, in the same place, with the same value.
//! Fields this build **adds** are permitted — that is the whole shape of an
//! additive change — and a test that forbade them would forbid the feature.
//! What it must never permit is a field that vanished, moved, or changed type.

use cairn_e2e::{baseline, Mcp, Sandbox};
use serde_json::{json, Value};

/// The one field whose **value** may legitimately move.
///
/// `estimated_tokens` measures the briefing that was actually assembled, and a
/// task-bound session now receives Level 0 work state it did not receive
/// before — so the measurement moves with the content, by design. The field
/// must still exist and still be a number.
///
/// The no-regression guarantee this does not weaken is FR-442's, which is about
/// a project with **no task, no warnings, no pins and no checkpoint**: nothing
/// is added there, so nothing may move there. That is a different baseline
/// (`briefing.json`) checked by a different test
/// (`us10_min_safe_context::no_regression`), and it compares the number exactly.
///
/// Everything else here — `budget`, `truncated`, `omitted_sections`,
/// `degraded`, and every briefing field — is compared for equality.
fn value_may_move(path: &str) -> bool {
    path.ends_with("estimated_tokens")
}

/// Every path in `recorded` must appear in `actual` with the same value.
///
/// Returns the paths that did not, so a failure names the field rather than
/// printing two documents and leaving the reader to diff them.
fn missing_or_changed(recorded: &Value, actual: &Value, path: &str, out: &mut Vec<String>) {
    match (recorded, actual) {
        (Value::Object(want), Value::Object(got)) => {
            for (key, value) in want {
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                match got.get(key) {
                    Some(actual) => missing_or_changed(value, actual, &child, out),
                    None => out.push(format!("{child} — the field is gone")),
                }
            }
        }
        (Value::Array(want), Value::Array(got)) => {
            if want.len() != got.len() {
                out.push(format!(
                    "{path} — {} entries, was {}",
                    got.len(),
                    want.len()
                ));
                return;
            }
            for (i, (w, g)) in want.iter().zip(got).enumerate() {
                missing_or_changed(w, g, &format!("{path}[{i}]"), out);
            }
        }
        (want, got) if want != got => {
            if value_may_move(path) {
                if want.is_number() != got.is_number() {
                    out.push(format!("{path} — {got}, was {want} (the type changed)"));
                }
                return;
            }
            out.push(format!("{path} — {got}, was {want}"));
        }
        _ => {}
    }
}

/// The JSON a tool result carries, wherever it carries it.
///
/// `cairn_context` answers with one rendered text blob — markdown for a person,
/// then a fenced JSON document. Comparing the blob verbatim would forbid every
/// addition, including the ones the contract requires: a task-bound session now
/// gets Level 0 work state it did not get before, and the human prose and the
/// token count move with it. What must not move is the **structure**, so the
/// document is pulled out and compared field by field.
fn embedded_json(value: &Value) -> Option<Value> {
    let text = value.as_str()?;
    let start = text.find("```json")? + "```json".len();
    let end = text[start..].find("```")? + start;
    serde_json::from_str(text[start..end].trim()).ok()
}

/// Compare one tool result, looking through any rendering.
fn compare_result(recorded: &Value, actual: &Value, out: &mut Vec<String>) {
    let (Some(want), Some(got)) = (recorded["content"].as_array(), actual["content"].as_array())
    else {
        missing_or_changed(recorded, actual, "", out);
        return;
    };
    if recorded["isError"] != actual["isError"] {
        out.push(format!(
            "isError — {}, was {}",
            actual["isError"], recorded["isError"]
        ));
    }
    for (i, (w, g)) in want.iter().zip(got).enumerate() {
        match (embedded_json(&w["text"]), embedded_json(&g["text"])) {
            // A rendered blob: compare the document it carries, not the prose
            // around it.
            (Some(w), Some(g)) => missing_or_changed(&w, &g, &format!("content[{i}]"), out),
            _ => missing_or_changed(&w["text"], &g["text"], &format!("content[{i}].text"), out),
        }
    }
}

/// `tools/list` is metadata, and Feature 003 is required to change some of it.
///
/// What must hold is narrower and more useful than equality: every tool, every
/// parameter and every enum value a Feature 001 caller knew about is still
/// there, with the same type; each description is **extended** rather than
/// rewritten; and nothing is removed. A caller written against the old surface
/// still constructs valid calls.
fn compare_tools_list(recorded: &Value, actual: &Value, out: &mut Vec<String>) {
    let want = recorded["tools"].as_array().expect("recorded tools");
    let got = actual["tools"].as_array().expect("tools");

    for tool in want {
        let name = tool["name"].as_str().unwrap_or_default();
        let Some(now) = got.iter().find(|t| t["name"] == tool["name"]) else {
            out.push(format!("{name} — the tool is gone"));
            continue;
        };

        // Extended, not rewritten. The recorded description, less its final
        // full stop, still opens the current one.
        let (before, after) = (
            tool["description"].as_str().unwrap_or_default(),
            now["description"].as_str().unwrap_or_default(),
        );
        if !after.starts_with(before.trim_end_matches('.')) {
            out.push(format!(
                "{name}.description was rewritten rather than extended:\n    was {before}\n    now {after}"
            ));
        }

        let want_props = tool["inputSchema"]["properties"]
            .as_object()
            .expect("properties");
        let got_props = now["inputSchema"]["properties"]
            .as_object()
            .expect("properties");
        for (field, schema) in want_props {
            let Some(current) = got_props.get(field) else {
                out.push(format!("{name}.{field} — the parameter is gone"));
                continue;
            };
            if schema["type"] != current["type"] {
                out.push(format!(
                    "{name}.{field} — type {}, was {}",
                    current["type"], schema["type"]
                ));
            }
            // Enum values may be **added** — that is how an action arrives —
            // but never removed, or a caller's existing value stops working.
            if let Some(values) = schema["enum"].as_array() {
                let now = current["enum"].as_array().cloned().unwrap_or_default();
                for value in values {
                    if !now.contains(value) {
                        out.push(format!("{name}.{field} no longer accepts {value}"));
                    }
                }
            }
        }
        // A parameter that used to be optional must not become required.
        let want_required = tool["inputSchema"]["required"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for required in now["inputSchema"]["required"]
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            if !want_required.contains(&required) {
                out.push(format!(
                    "{name} now requires {required}, which was optional"
                ));
            }
        }
    }
}

/// The recorded corpus, replayed call for call.
#[test]
fn every_pre_existing_field_is_unchanged() {
    let corpus: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("knowledge/baseline/mcp_calls.json"),
        )
        .expect("the recorded corpus"),
    )
    .expect("json");
    let calls = corpus["calls"].as_array().expect("recorded calls");
    assert!(
        calls.len() >= 12,
        "the corpus should cover the surface; found {}",
        calls.len()
    );

    let s = Sandbox::new();
    let cwd = s.repo_path().display().to_string();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));

    // The corpus was recorded against ids this run does not have, so the two
    // ids it threads between calls are substituted as they are learned. Every
    // other argument is replayed exactly as recorded.
    let mut task_id: Option<String> = None;
    let mut problems: Vec<String> = Vec::new();

    for (index, call) in calls.iter().enumerate() {
        let method = call["method"].as_str().unwrap_or_default();
        let recorded = &call["result"];

        let mut here = Vec::new();
        if method == "tools/list" {
            let actual = baseline::normalize(&mcp.call("tools/list", json!({})));
            compare_tools_list(recorded, &actual, &mut here);
            problems.extend(
                here.into_iter()
                    .map(|p| format!("call {index} (tools/list): {p}")),
            );
            continue;
        }

        let actual = {
            let name = call["params"]["name"].as_str().expect("a tool name");
            let mut arguments = call["params"]["arguments"].clone();
            if let (Some(object), Some(id)) = (arguments.as_object_mut(), task_id.as_ref()) {
                if object.get("task_id").is_some() {
                    object.insert("task_id".into(), json!(id));
                }
            }
            let result = mcp.tool_result(name, arguments, &cwd);
            if let Some(id) = result["content"][0]["text"]["task"]["id"].as_str() {
                task_id = Some(id.to_string());
            }
            baseline::normalize(&result)
        };

        compare_result(recorded, &actual, &mut here);
        for problem in here {
            let label = call["params"]["name"].as_str().unwrap_or(method);
            problems.push(format!("call {index} ({label}): {problem}"));
        }
    }

    assert!(
        problems.is_empty(),
        "Feature 001 callers would see a different answer:\n  {}",
        problems.join("\n  ")
    );
}

/// The surface is still exactly six tools (FR-495).
///
/// Asserted here as well as in the Feature 001 suite, because this is the test
/// a Feature 003 author would run, and a seventh tool is the failure this
/// feature was most at risk of.
#[test]
fn the_surface_is_still_exactly_six_tools_and_stop_is_still_absent() {
    let s = Sandbox::new();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));
    let listed = mcp.call("tools/list", json!({}));
    let tools = listed["tools"].as_array().expect("tools");

    let mut names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        vec![
            "cairn_context",
            "cairn_handoff",
            "cairn_remember",
            "cairn_search",
            "cairn_session",
            "cairn_task",
        ],
        "Feature 003 adds actions to the six, never a seventh tool"
    );

    // Every Feature 003 capability considered as a tool of its own — verify,
    // evidence, pattern, subject, checkpoint — is an action on one of these.
    for rejected in [
        "cairn_verify",
        "cairn_evidence",
        "cairn_pattern",
        "cairn_subject",
        "cairn_checkpoint",
    ] {
        assert!(
            !names.contains(&rejected),
            "`{rejected}` is an action on an existing tool, not a tool"
        );
    }

    let handoff = tools
        .iter()
        .find(|t| t["name"] == "cairn_handoff")
        .expect("cairn_handoff");
    let triggers = handoff["inputSchema"]["properties"]["trigger"]["enum"]
        .as_array()
        .expect("the trigger enum");
    assert!(
        !triggers.iter().any(|t| t == "stop"),
        "a turn checkpoint is not a handoff boundary: {triggers:?}"
    );
}

/// Every Feature 003 action reaches its tool through **one** discriminator.
///
/// No action takes a sub-operation (D70): an agent picks `action` and supplies
/// that action's parameters flat, rather than learning a second dispatch level
/// per tool.
#[test]
fn each_tool_has_one_discriminator_and_no_nested_operation() {
    let s = Sandbox::new();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));
    let listed = mcp.call("tools/list", json!({}));

    for tool in listed["tools"].as_array().expect("tools") {
        let name = tool["name"].as_str().unwrap_or_default();
        let properties = tool["inputSchema"]["properties"]
            .as_object()
            .expect("properties");

        // Exactly one enum named `action`, and nothing named like a second one.
        for suspect in ["operation", "sub_action", "subaction", "command", "method"] {
            assert!(
                !properties.contains_key(suspect),
                "{name} takes a second discriminator `{suspect}`"
            );
        }

        // Nothing nested: every parameter is a scalar or an array of scalars,
        // so an agent never has to construct a shape to call an action.
        for (field, schema) in properties {
            let kind = schema["type"].as_str().unwrap_or("string");
            assert_ne!(
                kind, "object",
                "{name}.{field} is an object; an action's parameters are flat"
            );
            if kind == "array" {
                assert_ne!(
                    schema["items"]["type"].as_str().unwrap_or("string"),
                    "object",
                    "{name}.{field} is an array of objects; an action's parameters are flat"
                );
            }
        }
    }
}

/// Every action the contract names is offered.
///
/// The other half of the six-tool rule: a surface that stayed at six tools by
/// leaving the new work unreachable would pass the count and fail the feature.
#[test]
fn every_feature_003_action_is_reachable() {
    let s = Sandbox::new();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));
    let listed = mcp.call("tools/list", json!({}));

    let actions = |name: &str| -> Vec<String> {
        listed["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name}"))["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.to_string())
            .collect()
    };

    for (tool, expected) in [
        (
            "cairn_remember",
            vec![
                "create",
                "supersede",
                "forget",
                "reinforce",
                "attach_evidence",
                "verify",
                "pin",
                "reconcile",
                "promote",
                "record_outcome",
            ],
        ),
        (
            "cairn_session",
            vec!["current", "start", "bind_task", "end", "checkpoint"],
        ),
        (
            "cairn_task",
            vec![
                "list",
                "get",
                "create",
                "update",
                "add_criterion",
                "update_criterion",
                "blocker",
                "readiness",
            ],
        ),
    ] {
        let offered = actions(tool);
        for action in expected {
            assert!(
                offered.iter().any(|a| a == action),
                "{tool} does not offer `{action}`: {offered:?}"
            );
        }
    }

    // The tools with no `action` discriminator gained parameters instead.
    let property = |tool: &str, field: &str| -> bool {
        listed["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .find(|t| t["name"] == tool)
            .unwrap_or_else(|| panic!("{tool}"))["inputSchema"]["properties"]
            .get(field)
            .is_some()
    };
    for (tool, field) in [
        ("cairn_context", "depth"),
        ("cairn_context", "include_patterns"),
        ("cairn_context", "explain"),
        ("cairn_search", "verification"),
        ("cairn_search", "authority"),
        ("cairn_search", "topic_key"),
        ("cairn_search", "as_of"),
        ("cairn_search", "include_patterns"),
        ("cairn_handoff", "include_checkpoint"),
    ] {
        assert!(property(tool, field), "{tool} is missing `{field}`");
    }

    // And `post_compaction` is reachable, which is how an agent with no
    // post-compaction event restores continuity itself.
    let reasons = listed["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["name"] == "cairn_context")
        .expect("cairn_context")["inputSchema"]["properties"]["reason"]["enum"]
        .as_array()
        .expect("the reason enum")
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>();
    assert!(
        reasons.contains(&"post_compaction"),
        "cairn_context must accept reason=post_compaction: {reasons:?}"
    );
    for existing in ["session_start", "continuation", "refresh"] {
        assert!(
            reasons.contains(&existing),
            "`{existing}` must keep working: {reasons:?}"
        );
    }
}

/// The MCP `instructions` string is the canonical contract, and stays bounded
/// (T129, FR-498, Feature 002 FR-129).
///
/// Asserted against the **running server** rather than against the renderer, so
/// a build that rendered the contract correctly and then served something else
/// still fails. The four Feature 003 obligations are checked by their substance:
/// an obligation an agent never reads is an obligation that does not exist.
#[test]
fn the_instructions_are_the_canonical_contract_and_stay_bounded() {
    let s = Sandbox::new();
    let mut mcp = Mcp::start(&s);
    let initialized = mcp.call("initialize", json!({}));
    let instructions = initialized["instructions"]
        .as_str()
        .expect("the server sends instructions");

    assert_eq!(
        instructions,
        cairn_integrate::render::Contract::canonical().mcp_instructions(),
        "the instructions must be generated from the one canonical contract source"
    );
    assert!(
        instructions.chars().count() <= cairn_integrate::render::CONTRACT_SIZE_BOUND,
        "the instructions are {} characters, over the {} bound",
        instructions.chars().count(),
        cairn_integrate::render::CONTRACT_SIZE_BOUND
    );

    let lower = instructions.to_lowercase();
    for (obligation, marker) in [
        ("give a fact a subject", "topic_key"),
        ("attach evidence rather than assert importance", "evidence"),
        ("reinforce a corroborating member", "corroborating"),
        ("record a pattern's outcome", "outcome"),
    ] {
        assert!(
            lower.contains(marker),
            "the contract does not state the obligation to {obligation}: {instructions}"
        );
    }

    // A generic client has neither hooks nor Skills, and the MCP rendering has
    // always said so by omission. Feature 003 must not have introduced one.
    assert!(!lower.contains("hook") && !lower.contains("skill"));
}

/// `memory show` keeps every field Feature 001 returned (FR-497).
///
/// Enriching the call with Feature 003's view once *replaced* the body with a
/// search result, which quietly dropped `project_id`, `origin_session_id`,
/// `updated_at` and `deleted_at` — and, because a search excludes deleted rows,
/// turned `memory show <forgotten-id>` into `not_found`. The response was
/// strictly better for a new reader and broken for an old one.
#[test]
fn memory_show_keeps_every_feature_001_field() {
    let s = Sandbox::new();
    let created = s.json(&[
        "memory",
        "add",
        "Errors are returned, never logged and swallowed",
        "--type",
        "convention",
        "--scope",
        "project",
    ]);
    let id = created["memory"]["id"].as_str().expect("id").to_string();

    let shown = s.json(&["memory", "show", &id])["memory"].clone();
    for field in [
        "id",
        "project_id",
        "type",
        "scope",
        "scope_key",
        "content",
        "state",
        "superseded_by_id",
        "origin_session_id",
        "local_only",
        "evidence",
        "created_at",
        "updated_at",
        "deleted_at",
    ] {
        assert!(
            shown.get(field).is_some(),
            "Feature 001 field `{field}` is gone from `memory show`: {shown}"
        );
    }
    // And the Feature 003 view is laid over it, not instead of it.
    for field in ["verification", "pinned", "importance"] {
        assert!(
            shown.get(field).is_some(),
            "Feature 003 field `{field}` is missing: {shown}"
        );
    }

    // A forgotten memory is a tombstone, not a 404: deletion is soft, and this
    // call has always answered for one.
    s.must(&["memory", "forget", &id]);
    let after = s.cairn(&["memory", "show", &id, "--json"]);
    assert!(
        after.ok(),
        "a forgotten memory stopped resolving: {} {}",
        after.stdout,
        after.stderr
    );
    let body: Value = serde_json::from_str(&after.stdout).expect("json");
    assert!(
        !body["data"]["memory"]["deleted_at"].is_null(),
        "the tombstone did not report when it was deleted: {body}"
    );
}

/// Every advertised action is *dispatched*, not merely listed.
///
/// `every_feature_003_action_is_reachable` reads the `tools/list` schema and
/// never issues a `tools/call`, so it passed green while seven of
/// `cairn_remember`'s ten actions, four of `cairn_task`'s eight and
/// `cairn_session`'s `checkpoint` fell through to `unknown action`. The schema
/// promised the whole Feature 003 write surface and the dispatch implemented
/// `create`, `supersede` and `forget` — so an agent reading the tool
/// definitions was told about capabilities the tool would refuse.
///
/// This calls each one. Most fail on a missing argument or a missing subject,
/// which is correct and is not what this asserts: the assertion is that no
/// advertised action is unknown to the dispatcher.
#[test]
fn every_advertised_action_is_dispatched() {
    let s = Sandbox::new();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));
    let listed = mcp.call("tools/list", json!({}));
    let cwd = s.repo_path().to_string_lossy().to_string();

    let tools = listed["tools"].as_array().expect("tools").clone();
    let mut unknown = Vec::new();

    for tool in &tools {
        let name = tool["name"].as_str().expect("name").to_string();
        let actions: Vec<String> = tool["inputSchema"]["properties"]["action"]["enum"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect();

        for action in actions {
            let result = mcp.call(
                "tools/call",
                json!({ "name": name, "arguments": { "cwd": cwd, "action": action } }),
            );
            let text = result["content"][0]["text"].as_str().unwrap_or_default();
            if text.contains("unknown action") {
                unknown.push(format!("{name}/{action}"));
            }
        }
    }

    assert!(
        unknown.is_empty(),
        "advertised but not dispatched: {unknown:?}"
    );

    // The detector detects. Without this, a change in how a refusal is
    // surfaced would turn this test into one that passes for every input.
    let control = mcp.call(
        "tools/call",
        json!({ "name": "cairn_remember",
                "arguments": { "cwd": cwd, "action": "definitely_not_an_action" } }),
    );
    assert!(
        control["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("unknown action"),
        "an unknown action no longer reports itself as one, so this test proves nothing: {control}"
    );
}

// ===========================================================================
// T188 / FR-527 / SC-430 — Feature 004 added actions and fields, not a tool
// ===========================================================================
//
// The six-tool count is the one thing in this feature that a single well-meaning
// commit could undo, and it would look like a kindness at the time: personal
// knowledge and team guidance are each plausibly "their own tool". They are not,
// and the reason is not aesthetic — six tools is what an agent can be expected to
// hold in its head, and every tool added is one more thing for it to pick wrongly
// among.
//
// So this feature reached the two new domains through *fields on existing
// actions*, and the tests below are what stop that from drifting back.

/// Still exactly six tools, and none of the names a seventh would plausibly take.
///
/// The forbidden list is the point: asserting a count alone passes on the day
/// `cairn_remember` is renamed and a seventh appears under the old name.
#[test]
fn feature_004_added_no_tool_and_none_of_the_names_one_would_have_taken() {
    let s = Sandbox::new();
    let mut mcp = Mcp::start(&s);
    mcp.call("initialize", json!({}));
    let listed = mcp.call("tools/list", json!({}));
    let tools: Vec<String> = listed["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .map(str::to_string)
        .collect();
    assert_eq!(
        tools.len(),
        6,
        "the surface is no longer six tools: {tools:?}"
    );

    const EXPECTED: &[&str] = &[
        "cairn_context",
        "cairn_search",
        "cairn_remember",
        "cairn_session",
        "cairn_task",
        "cairn_handoff",
    ];
    for name in EXPECTED {
        assert!(
            tools.iter().any(|t| t == name),
            "`{name}` is no longer advertised: {tools:?}"
        );
    }

    // Names a seventh tool for these domains would have taken.
    const FORBIDDEN: &[&str] = &[
        "cairn_personal",
        "cairn_team",
        "cairn_global",
        "cairn_knowledge",
        "cairn_promote",
        "cairn_ratify",
        "cairn_domain",
        "cairn_traits",
        "cairn_member",
        "cairn_user",
    ];
    for name in FORBIDDEN {
        assert!(
            !tools.iter().any(|t| t == name),
            "`{name}` was added as a tool; Feature 004 reaches its domains through \
             fields on the existing six (FR-527)"
        );
    }
}

/// The fields Feature 004 added are additive: absent, every one of them leaves
/// the pre-004 behaviour exactly as it was.
///
/// This is what "additive" has to mean in practice. A `domain` field that
/// defaulted to `personal`, or a `target` that defaulted to anything but
/// `pattern`, would be a silent behaviour change for every existing caller — and
/// the caller most affected is an agent running an older prompt that never
/// mentions either field.
#[test]
fn every_field_feature_004_added_is_absent_by_default_and_changes_nothing() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace")
            .join("crates/cairn/src/mcp.rs"),
    )
    .expect("read mcp.rs");

    // Each new field is advertised, so this test fails if one is quietly dropped
    // as well as if one becomes required.
    for field in [
        "\"domain\"",
        "\"target\"",
        "\"applicability_facts\"",
        "\"domains\"",
        "\"depth\"",
    ] {
        assert!(
            source.contains(field),
            "{field} is no longer advertised on any tool, so callers cannot reach \
             the capability it was added for"
        );
    }

    // And none of them is in a `required` list: `required` names only what a
    // pre-004 caller already sent.
    for required in source.match_indices("\"required\":") {
        let tail = &source[required.0..];
        let end = tail.find(']').map(|i| i + 1).unwrap_or(tail.len());
        let clause = &tail[..end];
        for field in [
            "domain",
            "target",
            "applicability_facts",
            "domains",
            "depth",
        ] {
            assert!(
                !clause.contains(&format!("\"{field}\"")),
                "`{field}` is required, which breaks every pre-004 caller: {clause}"
            );
        }
    }
}
