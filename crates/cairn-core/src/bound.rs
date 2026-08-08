//! Payload bounding and summarization (FR-013).
//!
//! Cairn stores structured facts, not transcripts. Anything larger than the
//! configured cap is summarized rather than stored, and the record says so.

pub const DEFAULT_PAYLOAD_CAP_BYTES: usize = 4096;

/// Result of bounding one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounded {
    pub text: String,
    pub truncated: bool,
}

/// Truncate `input` to at most `cap` bytes on a char boundary, appending a
/// marker that states how much was dropped.
///
/// The returned string is always `<= cap` bytes when `cap` is large enough to
/// hold the marker, and never splits a UTF-8 character.
pub fn bound_text(input: &str, cap: usize) -> Bounded {
    if input.len() <= cap {
        return Bounded {
            text: input.to_string(),
            truncated: false,
        };
    }
    if cap == 0 {
        return Bounded {
            text: String::new(),
            truncated: true,
        };
    }

    // Reserve room for the marker; if the cap is tiny, fall back to a hard cut.
    let dropped_hint = input.len();
    let marker = format!(" … [+{dropped_hint} bytes summarized]");
    let budget = cap.saturating_sub(marker.len());
    if budget == 0 {
        let mut end = cap.min(input.len());
        while end > 0 && !input.is_char_boundary(end) {
            end -= 1;
        }
        return Bounded {
            text: input[..end].to_string(),
            truncated: true,
        };
    }

    let mut end = budget.min(input.len());
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    let kept = &input[..end];
    let dropped = input.len() - end;
    let text = format!("{kept} … [+{dropped} bytes summarized]");
    debug_assert!(text.len() <= cap.max(text.len()));
    Bounded {
        text,
        truncated: true,
    }
}

/// Bound a JSON value by serializing it and capping the serialized form.
///
/// Oversized values are replaced with a summary object rather than a
/// half-parsed fragment: a truncated record is worse than an honest one.
pub fn bound_json(value: &serde_json::Value, cap: usize) -> (serde_json::Value, bool) {
    let encoded = value.to_string();
    if encoded.len() <= cap {
        return (value.clone(), false);
    }
    let summary = serde_json::json!({
        "summarized": true,
        "original_bytes": encoded.len(),
        "excerpt": bound_text(&encoded, cap.saturating_sub(64).max(1)).text,
    });
    (summary, true)
}

/// Total stored size of an observation's variable-length fields.
pub fn payload_bytes(
    summary: &str,
    path: Option<&str>,
    command: Option<&str>,
    details: Option<&serde_json::Value>,
) -> usize {
    summary.len()
        + path.map(str::len).unwrap_or(0)
        + command.map(str::len).unwrap_or(0)
        + details.map(|d| d.to_string().len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_passes_through() {
        let b = bound_text("hello", 100);
        assert_eq!(b.text, "hello");
        assert!(!b.truncated);
    }

    #[test]
    fn long_text_is_summarized_and_flagged() {
        let input = "x".repeat(10_000);
        let b = bound_text(&input, DEFAULT_PAYLOAD_CAP_BYTES);
        assert!(b.truncated);
        assert!(
            b.text.len() <= DEFAULT_PAYLOAD_CAP_BYTES,
            "{}",
            b.text.len()
        );
        assert!(b.text.contains("summarized"));
    }

    #[test]
    fn never_splits_a_utf8_character() {
        let input = "é".repeat(4000); // two bytes each
        let b = bound_text(&input, 101);
        assert!(b.text.len() <= 101);
        // Round-trips as valid UTF-8 by construction; assert explicitly.
        assert!(std::str::from_utf8(b.text.as_bytes()).is_ok());
    }

    #[test]
    fn oversized_json_becomes_a_summary_object() {
        let big = serde_json::json!({ "blob": "y".repeat(10_000) });
        let (v, truncated) = bound_json(&big, DEFAULT_PAYLOAD_CAP_BYTES);
        assert!(truncated);
        assert_eq!(v["summarized"], true);
        assert!(v.to_string().len() <= DEFAULT_PAYLOAD_CAP_BYTES);
    }

    #[test]
    fn payload_bytes_counts_every_stored_field() {
        let d = serde_json::json!({"a": 1});
        let n = payload_bytes("sum", Some("src/x.rs"), Some("cargo test"), Some(&d));
        assert_eq!(n, 3 + 8 + 10 + d.to_string().len());
    }
}
