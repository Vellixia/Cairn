//! MCP prompts (v0.5.0 Sprint 24).
//!
//! Prompts are template-driven conversation starters the MCP client can
//! render (e.g. "ask the model to summarize pending drift"). Five canonical
//! prompts are listed in [`prompt_defs`].
//!
//! Each prompt is a [`PromptDef`] with a name + description + argument
//! schema. [`render_prompt`] returns the rendered template as
//! `{ "messages": [{ "role": "user", "content": { "type": "text", "text": "..." } }] }`.

use serde::Serialize;
use serde_json::{json, Value};

/// Argument schema for one prompt. Empty when the prompt takes no args.
#[derive(Debug, Clone, Serialize)]
pub struct PromptArg {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct PromptDef {
    pub name: &'static str,
    pub description: &'static str,
    pub arguments: &'static [PromptArg],
    pub template: fn(&Value) -> String,
}

/// Five canonical prompts. The set is locked for v0.5.0 - adding more is a
/// breaking change for clients that enumerate the prompt list.
pub fn prompt_defs() -> &'static [PromptDef] {
    &[
        PromptDef {
            name: "summarize-drift",
            description: "Render a chat message asking the model to summarise pending drift items.",
            arguments: &[PromptArg {
                name: "limit",
                description: "Maximum number of drift items to mention (default 5).",
                required: false,
            }],
            template: |args| {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(5);
                format!(
                    "You have {limit} drift items awaiting review in Cairn. \
                     Read them and produce a one-paragraph summary, then list \
                     the items you would approve, reject, or escalate to the user."
                )
            },
        },
        PromptDef {
            name: "remember-decision",
            description: "Compose a `remember` tool call from a free-form decision description.",
            arguments: &[PromptArg {
                name: "decision",
                description: "The decision text. Markdown OK.",
                required: true,
            }],
            template: |args| {
                let decision = args
                    .get("decision")
                    .and_then(Value::as_str)
                    .unwrap_or("(no decision provided)");
                format!(
                    "Capture this decision as a durable Cairn memory so future \
                     agents see it:\n\n> {decision}\n\nCall `remember` with kind=`decision`."
                )
            },
        },
        PromptDef {
            name: "what-do-we-know",
            description: "Boot a fresh-agent recap by asking for the top-3 most-relevant memories.",
            arguments: &[PromptArg {
                name: "topic",
                description: "What the agent is about to work on.",
                required: true,
            }],
            template: |args| {
                let topic = args
                    .get("topic")
                    .and_then(Value::as_str)
                    .unwrap_or("(unknown topic)");
                format!(
                    "Before you start on \"{topic}\", run `proactive_recall` \
                     and review the returned memories. If any look relevant, \
                     surface them to the user and ask for confirmation before \
                     applying their guidance."
                )
            },
        },
        PromptDef {
            name: "weekly-savings-report",
            description: "Compose a Markdown report summarising the past 7 days of token savings.",
            arguments: &[],
            template: |_args| {
                "Generate a weekly savings report by reading `cairn://savings/today` \
                 and the per-day entries from `/api/ledger` for the past 7 days. \
                 Render as a Markdown table with columns: date, assemblies, \
                 tokens_in, tokens_out, tokens_saved, savings_pct. End with a \
                 short paragraph highlighting the biggest wins and any regressions."
                    .to_string()
            },
        },
        PromptDef {
            name: "drift-triage",
            description: "Walk through pending drift items one at a time for explicit approval.",
            arguments: &[],
            template: |_args| {
                "Read `cairn://drift/pending` and walk through each item one at a \
                 time. For each item: read the proposed resolution, ask the user \
                 approve/reject, then call `/api/drift/{id}/approve` or \
                 `/api/drift/{id}/reject` accordingly. After every 5 items, \
                 summarise progress."
                    .to_string()
            },
        },
    ]
}

/// Render a prompt by name. Returns the JSON payload shaped per the MCP
/// `prompts/get` spec.
pub fn render_prompt(name: &str) -> Result<Value, String> {
    for p in prompt_defs() {
        if p.name == name {
            // Prompts take either a flat object (preferred) or nothing.
            let args = json!({});
            let text = (p.template)(&args);
            return Ok(json!({
                "description": p.description,
                "messages": [{
                    "role": "user",
                    "content": { "type": "text", "text": text }
                }]
            }));
        }
    }
    Err(format!("unknown prompt name: {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_canonical_prompts_are_listed() {
        assert_eq!(prompt_defs().len(), 5, "v0.5.0 success metric: 5 prompts");
    }

    #[test]
    fn render_prompt_returns_err_for_unknown_name() {
        let res = render_prompt("does-not-exist");
        assert!(res.is_err());
    }

    #[test]
    fn remember_decision_prompt_uses_arg_text() {
        let args = json!({ "decision": "we use Rust" });
        let mut found = false;
        for p in prompt_defs() {
            if p.name == "remember-decision" {
                let text = (p.template)(&args);
                assert!(text.contains("we use Rust"));
                found = true;
            }
        }
        assert!(found);
    }
}
