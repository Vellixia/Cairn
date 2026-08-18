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
