//! Debug-only contention reporting.
//!
//! When a SQLite write loses a race, the message that reaches the user —
//! "database is locked" — says nothing about *which* write, or at which stage
//! of the transaction. Chasing that by reasoning is how a fix ends up aimed at
//! the wrong statement, so the store reports it instead.
//!
//! Off unless `CAIRN_CONTENTION_LOG` names a file. Nothing here records
//! payloads, content, paths or identifiers: a contention report is an
//! operation name, a stage, and two integers.

use std::io::Write;
use std::sync::OnceLock;

/// Where in a write the contention appeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Acquiring the write lock with `BEGIN IMMEDIATE`.
    Begin,
    /// A statement inside an open transaction.
    Body,
    /// `COMMIT`.
    Commit,
    /// `ROLLBACK`.
    Rollback,
    /// A single statement in autocommit, outside any transaction.
    Autocommit,
    /// Reported by the catch-all, which sees the error but not the stage.
    Unknown,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Begin => "begin_immediate",
            Stage::Body => "statement",
            Stage::Commit => "commit",
            Stage::Rollback => "rollback",
            Stage::Autocommit => "autocommit",
            Stage::Unknown => "unknown",
        }
    }
}

fn sink() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var("CAIRN_CONTENTION_LOG")
            .ok()
            .filter(|p| !p.is_empty())
    })
    .as_deref()
}

/// True when contention reporting is switched on.
pub fn enabled() -> bool {
    sink().is_some()
}

/// The extended and primary result codes of a database error, if it has any.
pub fn codes(e: &sqlx::Error) -> Option<(i64, i64)> {
    let sqlx::Error::Database(db) = e else {
        return None;
    };
    let extended = db.code().and_then(|c| c.parse::<i64>().ok())?;
    Some((extended, extended & 0xff))
}

/// Whether SQLite is saying "try again": `SQLITE_BUSY` or `SQLITE_LOCKED`, in
/// any of their extended forms.
pub fn is_contention(e: &sqlx::Error) -> bool {
    matches!(codes(e), Some((_, 5 | 6)))
}

/// Record one contention event. A no-op unless reporting is switched on.
pub fn record(op: &str, stage: Stage, entity: &str, attempt: u32, e: &sqlx::Error) {
    let Some(path) = sink() else { return };
    let Some((extended, primary)) = codes(e) else {
        return;
    };

    let mut line = format!(
        "op={op} stage={} entity={entity} extended={extended} primary={primary} attempt={attempt}",
        stage.as_str()
    );
    // The catch-all knows the error but not who raised it; the stack does.
    if stage == Stage::Unknown {
        line.push_str(&format!(" frames=[{}]", store_frames()));
    }
    line.push('\n');

    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// The `cairn` frames of the current stack, innermost first, names only.
///
/// Needs `RUST_BACKTRACE=1`; without it this is empty and the report still
/// carries its codes.
fn store_frames() -> String {
    let bt = std::backtrace::Backtrace::force_capture().to_string();
    let mut seen = Vec::new();
    for line in bt.lines() {
        let line = line.trim();
        let Some(rest) = line.split_once(": ").map(|(_, r)| r) else {
            continue;
        };
        if !(rest.starts_with("cairn_store::") || rest.starts_with("cairnd::")) {
            continue;
        }
        let name = rest.split("::{{").next().unwrap_or(rest).to_string();
        if !seen.contains(&name) {
            seen.push(name);
        }
        if seen.len() >= 6 {
            break;
        }
    }
    seen.join(" <- ")
}
