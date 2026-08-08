//! Secret redaction (FR-049).
//!
//! Runs in the daemon *before* any write, so nothing sensitive is persisted
//! even briefly (contracts/agent-integration.md). The pattern set is
//! documented and extensible; it is a mechanism, not a guarantee of catching
//! every possible secret (spec Assumptions).

use regex::Regex;
use std::sync::OnceLock;

pub const REDACTED: &str = "[REDACTED]";

struct Patterns {
    /// Whole-match redaction: the matched span is replaced entirely.
    whole: Vec<Regex>,
    /// Capture-group redaction: group 1 is kept, group 2 is replaced.
    keyed: Vec<Regex>,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let whole = [
            // PEM private key blocks.
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
            // JSON Web Tokens.
            r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
            // Provider-shaped API keys.
            r"\b(?:sk|pk|rk)-[A-Za-z0-9_\-]{16,}",
            r"\bghp_[A-Za-z0-9]{20,}",
            r"\bgithub_pat_[A-Za-z0-9_]{20,}",
            r"\bxox[baprs]-[A-Za-z0-9\-]{10,}",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"\bASIA[0-9A-Z]{16}\b",
            r"\bAIza[0-9A-Za-z_\-]{30,}",
            // Bearer credentials.
            r"(?i)\bbearer\s+[A-Za-z0-9._\-]{16,}",
            // Credentials embedded in a connection string.
            r"(?i)\b(?:postgres|postgresql|mysql|mongodb(?:\+srv)?|redis|amqp|https?)://[^\s:/@]+:[^\s@]{3,}@",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static redaction pattern"))
        .collect();

        let keyed = [
            // KEY=value / "token": "value" assignments, however quoted.
            r#"(?i)((?:[A-Za-z0-9_\-]*(?:api[_\-]?key|secret|token|password|passwd|credential|private[_\-]?key)[A-Za-z0-9_\-]*)["']?\s*[=:]\s*["']?)([^\s"',;]{4,})"#,
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static redaction pattern"))
        .collect();

        Patterns { whole, keyed }
    })
}

/// Redact secret-shaped values in `input`.
///
/// Idempotent: redacting already-redacted text changes nothing.
pub fn redact(input: &str) -> String {
    let p = patterns();
    let mut out = input.to_string();
    for re in &p.whole {
        out = re.replace_all(&out, REDACTED).into_owned();
    }
    for re in &p.keyed {
        out = re
            .replace_all(&out, format!("${{1}}{REDACTED}"))
            .into_owned();
    }
    out
}

/// Redact in place, returning whether anything changed.
pub fn redact_opt(input: &Option<String>) -> Option<String> {
    input.as_deref().map(redact)
}

/// Redact every string inside a JSON value, keys untouched.
pub fn redact_json(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::String(s) => Value::String(redact(s)),
        Value::Array(a) => Value::Array(a.iter().map(redact_json).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), redact_json(v))).collect())
        }
        other => other.clone(),
    }
}

/// True when the text still contains something secret-shaped.
///
/// Used by tests to assert the mechanism, not to gate writes.
pub fn contains_secret(input: &str) -> bool {
    let p = patterns();
    p.whole.iter().any(|re| re.is_match(input))
        || p.keyed.iter().any(|re| {
            re.captures(input)
                .map(|c| c.get(2).map(|m| m.as_str() != REDACTED).unwrap_or(false))
                .unwrap_or(false)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_openai_shaped_key() {
        let s = "export OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz0123";
        let out = redact(s);
        assert!(!out.contains("abcdefghijklmnopqrstuvwxyz"), "{out}");
        assert!(out.contains(REDACTED));
    }

    #[test]
    fn redacts_github_token_and_aws_key() {
        let out = redact("ghp_0123456789abcdefghijABCDEF and AKIAIOSFODNN7EXAMPLE");
        assert!(!out.contains("ghp_0123456789"));
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[test]
    fn redacts_bearer_and_jwt() {
        let out = redact("Authorization: Bearer abcdefghijklmnop.qrstuvwx");
        assert!(out.contains(REDACTED));
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert!(!redact(jwt).contains("dozjgNryP4J3"));
    }

    #[test]
    fn redacts_private_key_block() {
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEow...\n-----END RSA PRIVATE KEY-----";
        let out = redact(pem);
        assert_eq!(out, REDACTED);
    }

    #[test]
    fn redacts_connection_string_credentials() {
        let out = redact("postgres://admin:hunter2hunter2@db.internal:5432/app");
        assert!(!out.contains("hunter2hunter2"), "{out}");
    }

    #[test]
    fn redacts_keyed_assignment_but_keeps_the_key_name() {
        let out = redact(r#"{"password": "correct-horse-battery"}"#);
        assert!(out.contains("password"));
        assert!(!out.contains("correct-horse-battery"));
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let s = "cargo test --workspace failed in crates/cairn-store";
        assert_eq!(redact(s), s);
    }

    #[test]
    fn is_idempotent() {
        let once = redact("token=abcdefghijklmnop");
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn redacts_inside_json_values_only() {
        let v = serde_json::json!({"api_key": "sk-abcdefghijklmnopqrstuvwxyz0123", "n": 3});
        let out = redact_json(&v);
        assert_eq!(out["n"], 3);
        assert!(out["api_key"].as_str().unwrap().contains(REDACTED));
    }
}
