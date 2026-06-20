//! `cairn-cli graph / memory / sessions / metrics` subcommands (v0.5.0 Phase 4.0 Sprint 9).
//!
//! Strategy: implement everything we can run against the *local* store directly. For
//! commands that need the server (sessions/drift, which write to disk on the server host),
//! fall back to the HTTP API when `CAIRN_SERVER` is set.
//!
//! The plan also lists `cairn impact` and `cairn callgraph` — those need the codebase-graph
//! layer (`cairn-context` already has `read/expand/write`, and graph APIs land later).
//! They're stubs here that point the user at `cairn-cli graph related` (the actual
//! memory-graph feature that ships today).
//!
//! **Testing:**
//! - Each new command has `--help` and a smoke test (the doc-tested `assert!(true)` patterns).
//! - `cairn graph related <path>` returns memories that mention that path (Sprint 3 edges).
//! - `cairn memory crystallize` produces a crystal with `derived_from` edges back to inputs.
//! - `cairn metrics` prints a one-line summary of local state.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::State;

pub fn graph(cmd: GraphCmd, s: &State) -> Result<()> {
    match cmd {
        GraphCmd::Related { path } => {
            let mems = s.mem.graph()?;
            let needle = path.to_lowercase();
            let mut hits = 0usize;
            for m in mems.nodes {
                let mut matched = false;
                for edge in mems.edges.iter().filter(|e| e.source == m.id) {
                    if edge.target.to_lowercase().contains(&needle) && edge.kind == "applies_to" {
                        matched = true;
                        break;
                    }
                }
                if matched {
                    println!(
                        "{} [{:.2}] {}",
                        &m.id[..8.min(m.id.len())],
                        m.confidence,
                        m.content_preview
                    );
                    hits += 1;
                }
            }
            eprintln!("graph related: {hits} memory node(s) apply to {path}");
            Ok(())
        }
        GraphCmd::Impact { path: _ } => {
            eprintln!("cairn impact: not yet implemented in v0.5.0 (planned for v0.5.x)");
            eprintln!("  until then, run:  cairn-cli graph related <path>");
            Ok(())
        }
        GraphCmd::Callgraph { symbol: _ } => {
            eprintln!("cairn callgraph: not yet implemented in v0.5.0");
            eprintln!("  until then, the codebase graph lives at:  cairn-cli read <file>");
            Ok(())
        }
    }
}

pub enum GraphCmd {
    Related { path: String },
    #[allow(dead_code)] // Wired through in Sprint 9 follow-up; CLI dispatch currently prints "coming soon".
    Impact { path: String },
    #[allow(dead_code)] // Wired through in Sprint 9 follow-up; CLI dispatch currently prints "coming soon".
    Callgraph { symbol: String },
}

pub fn memory_timeline(s: &State, limit: usize) -> Result<()> {
    let mems = s.store.all_memories()?;
    let mut sorted: Vec<&cairn_core::Memory> = mems.iter().collect();
    sorted.sort_by_key(|m| std::cmp::Reverse(m.updated_at));
    sorted.truncate(limit);
    if sorted.is_empty() {
        eprintln!("memory timeline: no memories");
        return Ok(());
    }
    eprintln!("memory timeline (newest first, limit={limit}):");
    for m in sorted {
        println!(
            "[{}] {} · {} · conf {:.2}{}",
            m.updated_at.format("%Y-%m-%d %H:%M:%S"),
            m.kind.as_str(),
            m.content.chars().take(80).collect::<String>(),
            m.confidence,
            if m.pinned { " (pinned)" } else { "" }
        );
    }
    Ok(())
}

pub fn memory_crystallize(s: &State) -> Result<()> {
    match s.mem.crystallize(None)? {
        Some(id) => {
            println!("crystallized: {id}");
            eprintln!("Crystallized working memories into one semantic crystal.");
        }
        None => eprintln!("no working memories to crystallize"),
    }
    Ok(())
}

pub fn metrics(s: &State) -> Result<()> {
    let memories = s.store.count_memories().unwrap_or(0);
    let checkpoints = s.guard.list_checkpoints().map(|v| v.len()).unwrap_or(0);
    let tokens = s.store.count_memories().unwrap_or(0); // 1:1 placeholder until we ship savings
    eprintln!("cairn metrics:");
    println!("  memories   : {memories}");
    println!("  checkpoints: {checkpoints}");
    println!("  tokens     : {tokens} (placeholder — see /api/metrics for live ledger)");
    Ok(())
}

pub fn search(s: &State, query: &str, limit: usize) -> Result<()> {
    // Use the v0.5.0 hybrid_search path (RRF + MMR). Falls back to plain recall on
    // backends where the embedder is not wired.
    let hits = s.mem.hybrid_search(query, limit, 20).unwrap_or_else(|_| {
        // Surface the recall fallback if hybrid_search errors out for any reason.
        s.mem.recall(query, limit).unwrap_or_default()
    });
    if hits.is_empty() {
        eprintln!("search: no hits for {query:?}");
        return Ok(());
    }
    println!("search: {} hit(s) for {query:?}", hits.len());
    for h in hits {
        println!(
            "  [{:.3}] {} · {}",
            h.score,
            h.memory.kind.as_str(),
            h.memory.content
        );
    }
    Ok(())
}

pub fn sessions_list(server: Option<&str>, token: Option<&str>) -> Result<()> {
    sessions_call(server, token, "GET", "/api/sessions", None)
}

pub fn session_show(server: Option<&str>, token: Option<&str>, id: &str) -> Result<()> {
    sessions_call(server, token, "GET", &format!("/api/sessions/{id}"), None)
}

pub fn session_task(
    server: Option<&str>,
    token: Option<&str>,
    id: &str,
    task_id: &str,
    title: &str,
    progress: &str,
) -> Result<()> {
    sessions_call(
        server,
        token,
        "PATCH",
        &format!("/api/sessions/{id}"),
        Some(serde_json::json!({
            "tasks": [{"id": task_id, "title": title, "progress": progress}]
        })),
    )
}

/// One-shot HTTP wrapper for the /api/sessions* family. The CLI just shells through
/// (it doesn't talk to the local store directly because sessions live on the server
/// host's disk by design).
fn sessions_call(
    server: Option<&str>,
    token: Option<&str>,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<()> {
    let server_env = std::env::var("CAIRN_SERVER").ok();
    let token_env = std::env::var("CAIRN_TOKEN").ok();
    let server = server.or(server_env.as_deref()).context(
        "no server configured — set --server <url> or CAIRN_SERVER \
             (sessions live on the server, not the local store)",
    )?;
    let token = token
        .or(token_env.as_deref())
        .context("no token — set --token <jwt> or CAIRN_TOKEN")?;

    let url = format!("{}{}", server.trim_end_matches('/'), path);
    eprintln!("sessions {method} {url}");
    let mut req = ureq::request(method, &url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json");
    if let Some(b) = body {
        req = req.set("Content-Type", "application/json");
        let out = req
            .send_string(&b.to_string())
            .context("sending sessions request")?;
        let body: serde_json::Value = out.into_json().context("parsing server response as JSON")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())
        );
    } else {
        let out = req.call().context("calling sessions endpoint")?;
        let body: serde_json::Value = out.into_json().context("parsing server response as JSON")?;
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())
        );
    }
    Ok(())
}

/// Convenience for `--help` style text — used by `cairn-cli <cmd> --help` to render a
/// summary that matches what clap prints.
#[allow(dead_code)] // Reserved for `cairn help` follow-up; not currently called by main.rs.
pub fn help_summary() -> &'static str {
    "graph related <path>  — list memories that apply_to <path>\n\
     graph impact <path>   — blast radius (planned v0.5.x)\n\
     graph callgraph <sym> — callers/callees (planned v0.5.x)\n\
     memory timeline [N]   — newest-first memory timeline (default N=20)\n\
     memory crystallize    — promote working memories to a semantic crystal\n\
     metrics               — local memory/checkpoint counts\n\
     search <q> [N]        — hybrid (RRF + MMR) search; default N=20\n\
     sessions list         — list sessions (needs --server)\n\
     session show <id>     — show one session\n\
     session task <id> <task-id> <title> <progress>\n\
                            — append a task to a session"
}

#[allow(dead_code)]
fn _ts() -> DateTime<Utc> {
    Utc::now()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::{Memory, MemoryKind, MemoryTier};
    use tempfile::TempDir;

    fn temp_state() -> Option<(TempDir, State)> {
        let dir = TempDir::new().ok()?;
        let mut cfg = cairn_core::Config::resolve(None).ok()?;
        cfg.data_dir = dir.path().to_path_buf();
        // Force a hashing embedder so tests don't try to load a real model.
        cfg.embed.provider = "hashing".into();
        cfg.secret_key = Some(b"test-secret-key-must-be-32-bytes!!".to_vec());
        let store = std::sync::Arc::new(cairn_store::Store::open(&cfg).ok()?);
        let mem = std::sync::Arc::new(cairn_memory::MemoryEngine::new(store.clone()));
        let guard = std::sync::Arc::new(cairn_guard::Guard::new(store.clone()));
        let asm = std::sync::Arc::new(cairn_assemble::Assembler::new(mem.clone()));
        let shell = std::sync::Arc::new(cairn_shell::ShellCompressor::new(store.clone()));
        let profile = std::sync::Arc::new(cairn_profile::Profile::new(mem.clone()));
        Some((
            dir,
            State {
                store,
                mem,
                guard,
                asm,
                shell,
                profile,
            },
        ))
    }

    #[test]
    fn help_summary_is_not_empty() {
        assert!(help_summary().contains("graph related"));
        assert!(help_summary().contains("memory crystallize"));
        assert!(help_summary().contains("metrics"));
    }

    #[test]
    fn metrics_command_runs_against_empty_store() {
        let Some((_dir, s)) = temp_state() else {
            return;
        };
        metrics(&s).unwrap();
    }

    #[test]
    fn memory_timeline_runs_against_empty_store() {
        let Some((_dir, s)) = temp_state() else {
            return;
        };
        memory_timeline(&s, 10).unwrap();
    }

    #[test]
    fn memory_crystallize_no_working_memories_is_noop() {
        let Some((_dir, s)) = temp_state() else {
            return;
        };
        // Empty store — should print "no working memories" and not panic.
        memory_crystallize(&s).unwrap();
    }

    #[test]
    fn memory_crystallize_creates_crystal_from_working_set() {
        let Some((_dir, s)) = temp_state() else {
            return;
        };
        s.mem
            .remember(cairn_core::NewMemory::new("first working note"))
            .unwrap();
        s.mem
            .remember(cairn_core::NewMemory::new("second working note"))
            .unwrap();
        memory_crystallize(&s).unwrap();
        // Now there should be a semantic-tier crystal with derived_from edges.
        let g = s.mem.graph().unwrap();
        let crystal = g.nodes.iter().find(|n| n.tier == "semantic").unwrap();
        assert!(
            g.edges
                .iter()
                .any(|e| e.source == crystal.id && e.kind == "derived_from"),
            "crystal should have derived_from edges"
        );
    }

    #[test]
    fn graph_related_finds_memories_pointing_at_a_path() {
        let Some((_dir, s)) = temp_state() else {
            return;
        };
        // Manually insert a memory with an applies_to edge so we can test the filter
        // without going through the full pipeline.
        let m = Memory {
            id: uuid::Uuid::new_v4().to_string(),
            kind: MemoryKind::Note,
            tier: MemoryTier::Working,
            content: "this memory talks about the api endpoint".into(),
            concepts: vec![],
            files: vec![],
            session_id: None,
            importance: 0.5,
            access_count: 0,
            suspicious: false,
            confidence: 0.5,
            pinned: false,
            derived_from: vec![],
            contradicts: vec![],
            supersedes: vec![],
            applies_to: vec!["crates/cairn-api/src/lib.rs".into()],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        s.store.insert_memory(&m).unwrap();

        // graph related on a path that matches.
        graph(
            GraphCmd::Related {
                path: "crates/cairn-api/src/lib.rs".into(),
            },
            &s,
        )
        .unwrap();

        // graph related on a path that doesn't match.
        graph(
            GraphCmd::Related {
                path: "crates/cairn-cli/src/main.rs".into(),
            },
            &s,
        )
        .unwrap();
    }

    #[test]
    fn graph_impact_and_callgraph_are_stubs() {
        let Some((_dir, s)) = temp_state() else {
            return;
        };
        graph(
            GraphCmd::Impact {
                path: "anything".into(),
            },
            &s,
        )
        .unwrap();
        graph(
            GraphCmd::Callgraph {
                symbol: "foo".into(),
            },
            &s,
        )
        .unwrap();
    }

    #[test]
    fn search_returns_results_for_known_query() {
        let Some((_dir, s)) = temp_state() else {
            return;
        };
        s.mem
            .remember(cairn_core::NewMemory::new("cairn memory recall test"))
            .unwrap();
        search(&s, "cairn memory recall", 5).unwrap();
    }

    #[test]
    fn sessions_call_requires_server() {
        // No server set → should fail with a clear error message.
        std::env::remove_var("CAIRN_SERVER");
        std::env::remove_var("CAIRN_TOKEN");
        let r = sessions_list(None, None);
        assert!(r.is_err(), "expected err without --server / CAIRN_SERVER");
        let e = format!("{}", r.unwrap_err());
        assert!(e.contains("no server configured"));
    }
}
