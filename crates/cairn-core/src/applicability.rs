//! Which projects a personal or team record applies to (FR-434–FR-436, D410–D412).
//!
//! Applicability answers "does this record apply to *this* project". It is a
//! different question from a record's own `topic_key`, which answers "what is
//! this record about", and FR-570 forbids conflating them: the predicate below
//! never reads a `topic_key`, and `derive_subject`'s reconciliation never reads
//! an applicability fact.
//!
//! The kind vocabulary is closed to `language | tool`
//! ([`ApplicabilityKind`]). The *value* is not — the set of language and tool
//! names is open by nature — so a value is screened by
//! [`crate::validate::validate_global_content`] rather than by an enum
//! (FR-578). Reading "closed vocabulary" as "this field cannot carry a project
//! name" is exactly the mistake FR-579 exists to prevent.

#[cfg(test)]
use crate::domain::ApplicabilityKind;

/// Why an applicability value was refused.
///
/// Carries no offending text, for the same reason
/// [`crate::validate::GlobalContentRejection`] carries none: a type with
/// nowhere to put the value cannot leak it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicabilityRejection {
    /// Failed `normalize_value_key`, or is not `[a-z0-9_]{1,64}` afterwards.
    InvalidValue,
}

/// Normalize and constrain one applicability value (FR-446, D410).
///
/// Tighter than a memory's `value_key`, and **deliberately not built on it**. A
/// value here names one discrete fact — "rust", "cargo", "graphql" — and
/// nothing else is representable. A value that fails causes the *creation* to
/// be refused; it is never silently dropped, truncated, or stored with a null
/// kind.
///
/// This used to delegate to `normalize_value_key` and then re-check the result,
/// which worked only because that function did no separator folding: `has
/// space` and `path/like` survived unchanged and failed the `[a-z0-9_]` check
/// afterwards. Feature 005 folds separators in value keys (FR-796a), so the
/// same delegation would now coerce those into `has_space` and `path_like` and
/// accept them — turning a refusal into exactly the silent repair this function
/// exists to prevent. The rule is therefore stated here directly: case and
/// Unicode form are normalized, and everything else must already be a single
/// `[a-z0-9_]` token.
pub fn normalize_applicability_value(value: &str) -> Result<String, ApplicabilityRejection> {
    use unicode_normalization::UnicodeNormalization;
    let normalized: String = value.nfc().collect::<String>().to_lowercase();
    let acceptable = !normalized.is_empty()
        && normalized.chars().count() <= 64
        && normalized
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if acceptable {
        Ok(normalized)
    } else {
        Err(ApplicabilityRejection::InvalidValue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vocabulary is exactly two members, and `topic` is not one of them
    /// (FR-569, D439). A third kind that could never be derived from a working
    /// tree would silently make every record carrying it inapplicable
    /// everywhere, which is a filter that excludes without saying so.
    #[test]
    fn the_kind_vocabulary_is_exactly_language_and_tool() {
        assert_eq!(ApplicabilityKind::ALL.len(), 2);
        let names: Vec<&str> = ApplicabilityKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(names, vec!["language", "tool"]);
        assert!("topic".parse::<ApplicabilityKind>().is_err());
    }

    /// A value outside `[a-z0-9_]{1,64}` is **refused**, not silently dropped
    /// or truncated (FR-446). Asserting only that valid values are accepted
    /// would pass on an implementation that accepted everything.
    #[test]
    fn an_unrepresentable_value_is_refused_rather_than_coerced() {
        assert!(normalize_applicability_value("rust").is_ok());
        assert!(normalize_applicability_value("Rust").is_ok(), "lowercased");
        for bad in [
            "",
            "   ",
            "has space",
            "has-dash",
            "path/like",
            "dots.in.it",
            "UPPER!",
            "@scope",
        ] {
            assert!(
                normalize_applicability_value(bad).is_err(),
                "{bad:?} was accepted"
            );
        }
        assert!(normalize_applicability_value(&"a".repeat(65)).is_err());
        assert!(normalize_applicability_value(&"a".repeat(64)).is_ok());
    }
}
