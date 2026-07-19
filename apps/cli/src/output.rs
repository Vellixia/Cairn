//! T023: envelope rendering + exit codes. Stdout carries the envelope only;
//! diagnostics go to stderr. Human mode NEVER prints resume tokens (FR-029).

use cairn_protocol::{CliEnvelope, ErrorBody};

/// Emit the result and return the process exit code.
pub fn emit(command: &str, json: bool, result: Result<serde_json::Value, ErrorBody>) -> i32 {
    match result {
        Ok(mut data) => {
            let exit = extra_exit_code(command, &data);
            if json {
                let env = CliEnvelope::ok(command, &data);
                println!(
                    "{}",
                    serde_json::to_string(&env).expect("serializable envelope")
                );
            } else {
                redact_tokens(&mut data);
                print_human(command, &data);
            }
            exit
        }
        Err(err) => {
            let exit = err.code.exit_code();
            if json {
                let env = CliEnvelope::err(command, err);
                println!(
                    "{}",
                    serde_json::to_string(&env).expect("serializable envelope")
                );
            } else {
                eprintln!("error [{}]: {}", code_str(&err), err.message);
            }
            exit
        }
    }
}

fn code_str(err: &ErrorBody) -> String {
    serde_json::to_value(err.code)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "INTERNAL".into())
}

/// Ambiguous session resolution is a success payload with exit code 4.
fn extra_exit_code(command: &str, data: &serde_json::Value) -> i32 {
    if command == "session.show"
        && data.get("resolution").and_then(|v| v.as_str()) == Some("ambiguous")
    {
        return 4;
    }
    0
}

/// Strip token material from anything a human terminal will see.
fn redact_tokens(data: &mut serde_json::Value) {
    if let Some(obj) = data.as_object_mut() {
        if obj.contains_key("resume_token") && !obj["resume_token"].is_null() {
            obj["resume_token"] = serde_json::Value::String("<hidden>".into());
        }
        for (_, v) in obj.iter_mut() {
            redact_tokens(v);
        }
    } else if let Some(arr) = data.as_array_mut() {
        for v in arr {
            redact_tokens(v);
        }
    }
}

fn print_human(command: &str, data: &serde_json::Value) {
    match command {
        "init" => {
            let repo = &data["repository"];
            let created = data["created"].as_bool().unwrap_or(false);
            let outcome = data["identity_outcome"].as_str().unwrap_or("?");
            if created {
                println!(
                    "Registered repository at {} (id {}, identity {outcome})",
                    repo["canonical_path"].as_str().unwrap_or("?"),
                    repo["repository_id"].as_str().unwrap_or("?"),
                );
            } else {
                println!(
                    "Already registered (id {}, identity {outcome})",
                    repo["repository_id"].as_str().unwrap_or("?"),
                );
            }
            if outcome == "new_after_marker_loss" {
                eprintln!(
                    "warning: identity markers were missing and no unique prior record matched; \
                     a NEW identity was created — prior history was NOT attached"
                );
            }
        }
        "status" => {
            let branch =
                data["branch"]
                    .as_str()
                    .unwrap_or(if data["detached"].as_bool().unwrap_or(false) {
                        "(detached)"
                    } else {
                        "?"
                    });
            println!("root:      {}", data["root"].as_str().unwrap_or("?"));
            println!("branch:    {branch}");
            println!(
                "head:      {}",
                data["head_commit"].as_str().unwrap_or("(unborn)")
            );
            if let Some(r) = data["default_remote"].as_object() {
                println!(
                    "remote:    {} ({})",
                    r["name"].as_str().unwrap_or("?"),
                    r["url"].as_str().unwrap_or("?")
                );
            }
            if let Some(op) = data["in_progress"].as_str() {
                println!("operation: {op} in progress");
            }
            print_changes("staged", &data["staged"]);
            print_changes("unstaged", &data["unstaged"]);
            if let Some(u) = data["untracked"].as_array() {
                println!("untracked: {}", u.len());
                for p in u.iter().take(10) {
                    println!("  ? {}", p.as_str().unwrap_or("?"));
                }
            }
            if let Some(ig) = data["ignored_summary"].as_object() {
                println!(
                    "ignored:   {} files{}",
                    ig["total_count"].as_u64().unwrap_or(0),
                    if ig["truncated"].as_bool().unwrap_or(false) {
                        " (truncated count)"
                    } else {
                        ""
                    }
                );
            }
        }
        "daemon.status" => {
            println!("version:   {}", data["version"].as_str().unwrap_or("?"));
            println!("pid:       {}", data["pid"]);
            println!("uptime:    {}s", data["uptime_seconds"]);
            println!(
                "db:        {} (healthy: {})",
                data["db_path"].as_str().unwrap_or("?"),
                data["db_healthy"]
            );
            println!("watched:   {}", data["watched_repositories"]);
            println!("sessions:  {} active", data["active_sessions"]);
        }
        "session.show" => {
            if data["resolution"].as_str() == Some("ambiguous") {
                println!("Multiple live sessions — specify --session or --agent-instance:");
                if let Some(c) = data["candidates"].as_array() {
                    for s in c {
                        println!(
                            "  {}  {}  {}  instance {}",
                            s["session_id"].as_str().unwrap_or("?"),
                            s["state"].as_str().unwrap_or("?"),
                            s["agent_type"].as_str().unwrap_or("?"),
                            s["agent_instance_id"].as_str().unwrap_or("?"),
                        );
                    }
                }
            } else {
                print_session(&data["session"]);
            }
        }
        "session.start" | "session.stop" | "session.reattach" => {
            print_session(&data["session"]);
            if command == "session.start" || command == "session.reattach" {
                if data["resume_token"].is_string() {
                    eprintln!("resume token issued — rerun with --json to capture, or use CAIRN_RESUME_TOKEN");
                }
                if let Some(outcome) = data["outcome"].as_str() {
                    println!("outcome:   {outcome}");
                }
            }
        }
        "session.heartbeat" => {
            println!("state:     {}", data["state"].as_str().unwrap_or("?"));
            println!(
                "lease:     until {}",
                data["lease_expires_at"].as_str().unwrap_or("?")
            );
        }
        "status.ignored" => {
            if let Some(paths) = data["paths"].as_array() {
                for p in paths {
                    println!("{}", p.as_str().unwrap_or("?"));
                }
            }
            if let Some(next) = data["next_cursor"].as_str() {
                eprintln!("more results — continue with --cursor {next}");
            }
        }
        _ => println!("{}", serde_json::to_string_pretty(data).unwrap_or_default()),
    }
}

fn print_changes(label: &str, arr: &serde_json::Value) {
    if let Some(items) = arr.as_array() {
        println!("{label}:    {}", items.len());
        for c in items.iter().take(10) {
            println!(
                "  {} {}",
                c["status"].as_str().unwrap_or("?"),
                c["path"].as_str().unwrap_or("?"),
            );
        }
    }
}

fn print_session(s: &serde_json::Value) {
    println!("session:   {}", s["session_id"].as_str().unwrap_or("?"));
    println!("state:     {}", s["state"].as_str().unwrap_or("?"));
    println!(
        "agent:     {} (instance {})",
        s["agent_type"].as_str().unwrap_or("?"),
        s["agent_instance_id"].as_str().unwrap_or("?")
    );
    println!("started:   {}", s["started_at"].as_str().unwrap_or("?"));
    println!(
        "start fp:  {}",
        s["start_snapshot"]["snapshot_fp"].as_str().unwrap_or("?")
    );
    println!(
        "current fp:{}",
        s["current_snapshot"]["snapshot_fp"].as_str().unwrap_or("?")
    );
}
