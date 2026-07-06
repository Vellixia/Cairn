//! `cairn statusline` - one fast line of ambient state for Claude Code's `statusLine` setting
//! (C-5): permanent visibility into memory count, anchor, token savings, and offline-hook
//! backlog, right in the terminal, without opening the dashboard.
//!
//! Must never hang or panic the statusline UI: every network call is tightly timeout-bounded
//! and every failure degrades the line gracefully (never an error, never a stack trace).

use std::time::Duration;

/// Tighter than the client default (4000ms) - a statusline command runs on every prompt-bar
/// render, so it needs to fail fast rather than make the UI feel laggy when the server's down.
const STATUSLINE_TIMEOUT_MS: u64 = 1200;

pub fn run() {
    println!("{}", build_line());
}

fn build_line() -> String {
    let (project_id, _) = crate::project::detect_project();
    let resolved = crate::config::resolve(project_id.as_deref());
    let spool_depth = crate::spool::depth();

    let Some((server, _)) = resolved.server else {
        return format!("\u{26f0} cairn: not configured | spool {spool_depth}");
    };
    let token = resolved.token.as_ref().map(|(t, _)| t.as_str());
    let client = crate::http::ApiClient::with_timeout(
        &server,
        token,
        Duration::from_millis(STATUSLINE_TIMEOUT_MS),
    );

    let stats: Option<serde_json::Value> = client
        .get("/api/stats")
        .call()
        .ok()
        .and_then(|r| r.into_json().ok());

    let Some(stats) = stats else {
        return format!("\u{26f0} cairn: offline | spool {spool_depth}");
    };

    let memories = stats.get("memories").and_then(|v| v.as_u64()).unwrap_or(0);
    let anchor = stats
        .get("anchor")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());

    // Best-effort second call for token savings - if it's slow or fails, drop that segment
    // rather than block or blank the whole line.
    let tokens_saved = client
        .get("/api/metrics")
        .call()
        .ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .and_then(|m| {
            let savings = m.get("savings")?;
            let wakeup = savings
                .get("wakeup_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let recall = savings
                .get("recall_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Some(wakeup + recall)
        });

    let mut parts = vec![format!("{memories} mem")];
    if let Some(a) = anchor {
        parts.push(format!("anchor: {}", truncate(a, 24)));
    }
    if let Some(saved) = tokens_saved {
        parts.push(format!("\u{2193}{} tok saved", format_count(saved)));
    }
    parts.push(format!("spool {spool_depth}"));

    format!("\u{26f0} {}", parts.join(" | "))
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    format!("{}...", s.chars().take(max_chars).collect::<String>())
}

fn format_count(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_count_abbreviates_thousands() {
        assert_eq!(format_count(0), "0");
        assert_eq!(format_count(999), "999");
        assert_eq!(format_count(1000), "1.0k");
        assert_eq!(format_count(18_200), "18.2k");
    }

    #[test]
    fn truncate_leaves_short_strings_alone() {
        assert_eq!(truncate("refactor web", 24), "refactor web");
    }

    #[test]
    fn truncate_shortens_long_strings_with_ellipsis() {
        let long = "a very long anchor goal that goes on and on";
        let out = truncate(long, 24);
        assert_eq!(out, format!("{}...", &long[..24]));
        assert!(out.chars().count() <= 27); // 24 + "..."
    }
}
