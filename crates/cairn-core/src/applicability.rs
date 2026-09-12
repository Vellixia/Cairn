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

use crate::domain::{ApplicabilityFact, ApplicabilityKind, ProjectTrait};
use std::collections::BTreeSet;

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

/// Does `record` apply to a project with `traits`? (FR-436, D412)
///
/// **AND across kinds, OR within a kind.** A record naming both
/// `language=rust` and `tool=docker` applies only to a project that is both. A
/// record naming `language=rust` and `language=python` applies to a project
/// that is either — two facts of one kind are alternatives, not a conjunction.
///
/// **No facts means universal** (FR-435). The empty set means "applies
/// everywhere", not "applies nowhere", and that default is what keeps "remember
/// this for me, everywhere" the simple case.
pub fn applies(record: &[ApplicabilityFact], traits: &[ProjectTrait]) -> bool {
    let kinds: BTreeSet<ApplicabilityKind> = record.iter().map(|f| f.kind).collect();
    kinds.into_iter().all(|kind| {
        record
            .iter()
            .filter(|f| f.kind == kind)
            .any(|f| traits.iter().any(|t| t.kind == kind && t.value == f.value))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(kind: ApplicabilityKind, value: &str) -> ApplicabilityFact {
        ApplicabilityFact {
            kind,
            value: value.to_string(),
        }
    }
    fn trait_(kind: ApplicabilityKind, value: &str) -> ProjectTrait {
        ProjectTrait {
            kind,
            value: value.to_string(),
        }
    }

    /// The worked truth table from `contracts/global-memory.md` §4, row by row.
    /// Project traits: `{language: rust, tool: cargo}`.
    #[test]
    fn the_match_predicate_matches_the_contracts_truth_table() {
        use ApplicabilityKind::{Language, Tool};
        let traits = [trait_(Language, "rust"), trait_(Tool, "cargo")];

        // 1. No facts means universal (FR-435).
        assert!(applies(&[], &traits));
        // 2. The one matching `language` fact.
        assert!(applies(&[fact(Language, "rust")], &traits));
        // 3. A non-matching sole fact of its kind.
        assert!(!applies(&[fact(Language, "python")], &traits));
        // 4. OR within a kind: either alternative suffices.
        assert!(applies(
            &[fact(Language, "python"), fact(Language, "rust")],
            &traits
        ));
        // 5. AND across kinds: both must be satisfied.
        assert!(applies(
            &[fact(Language, "rust"), fact(Tool, "cargo")],
            &traits
        ));
        // 6. AND across kinds, one kind unsatisfied.
        assert!(!applies(
            &[fact(Language, "rust"), fact(Tool, "docker")],
            &traits
        ));
    }

    /// A project with no derivable traits admits only universal records
    /// (SC-429). Asserted as a refusal, not only as an acceptance: the
    /// interesting direction is that a kind-restricted record is *excluded*.
    #[test]
    fn a_project_with_no_traits_admits_only_universal_records() {
        assert!(applies(&[], &[]));
        assert!(!applies(&[fact(ApplicabilityKind::Language, "rust")], &[]));
        assert!(!applies(&[fact(ApplicabilityKind::Tool, "cargo")], &[]));
    }

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
