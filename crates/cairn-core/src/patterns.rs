//! Reusable cross-project patterns: the pure parts (`contracts/patterns.md`).
//!
//! Normalization, the two digests, signal matching, and the trust ladder. No
//! storage, no clock, no identity — which is what lets the corpus assert them
//! directly and lets the gate in `cairn-store` be about *policy* rather than
//! about string handling.

use crate::domain::{PatternDiscovery, PatternOutcome, PatternTrust};

/// The most signals a pattern may carry (`contracts/patterns.md` §Anatomy).
pub const SIGNALS_MAX: usize = 16;
/// Longest single signal, in characters.
pub const SIGNAL_MAX_CHARS: usize = 128;

/// One symptom token or error signature, in comparable form.
///
/// NFC, lower-case, whitespace-collapsed, trailing punctuation stripped — the
/// same shape [`crate::knowledge::normalize_content`] produces, because a
/// signal is a fragment of the same kind of text and two normalizers would
/// eventually disagree.
///
/// `None` when nothing representable is left. An unrepresentable signal is
/// dropped rather than stored raw: a signal that cannot be compared cannot
/// match, and keeping it would inflate the count the specificity check reads.
pub fn normalize_signal(signal: &str) -> Option<String> {
    let normalized = crate::knowledge::normalize_content(signal);
    if normalized.is_empty() {
        return None;
    }
    Some(
        normalized
            .chars()
            .take(SIGNAL_MAX_CHARS)
            .collect::<String>()
            .trim()
            .to_string(),
    )
    .filter(|s| !s.is_empty())
}

/// The comparable signal set: normalized, de-duplicated, sorted, bounded.
///
/// Sorted because [`signal_digest`] is taken over it and two writers listing
/// the same signals in a different order describe the same pattern. Bounded
/// because a set nobody bounded is a match cost nobody bounded.
pub fn normalize_signals(signals: &[String]) -> Vec<String> {
    let mut out: Vec<String> = signals.iter().filter_map(|s| normalize_signal(s)).collect();
    out.sort();
    out.dedup();
    out.truncate(SIGNALS_MAX);
    out
}

/// The digest used for **both** matching and duplicate detection.
///
/// One representation, deliberately: two digests — one for "is this the same
/// pattern" and one for "does this pattern apply here" — could disagree, and
/// the disagreement would be invisible.
pub fn signal_digest(signals: &[String]) -> String {
    crate::digest(&normalize_signals(signals).join("\n"))
}

/// The other half of the duplicate key.
pub fn root_cause_digest(root_cause: &str) -> String {
    crate::knowledge::content_norm_digest(root_cause)
}

/// Words too common to distinguish one failure from another.
///
/// Short and fixed. This is not stemming and not a stopword *model*: it is the
/// handful of words that appear in almost every error message, listed so that
/// "could not" and "the file" cannot on their own make two unrelated problems
/// look like the same one.
const COMMON: &[&str] = &[
    "could", "cannot", "with", "that", "this", "from", "have", "been", "when", "while", "there",
    "which", "your", "into", "then", "than", "were", "will", "would", "should", "must", "does",
    "error", "failed", "failure", "fails", "unable", "please", "after", "before",
];

/// The distinguishing words in a signal set.
///
/// Matching compares **tokens**, not whole strings. A pattern's signals are
/// error signatures as someone wrote them down; a project's signals are the
/// error text Cairn recorded. Those are never character-for-character equal, so
/// a whole-string comparison would mean no pattern is ever suggested — the
/// feature would look implemented and do nothing.
///
/// Still not a similarity measure. Two tokens are the same string or they are
/// unrelated: no stemming, no distance, no embedding. What makes a match
/// meaningful is requiring several of them (`pattern_signals_min`), not making
/// any one of them fuzzy (FR-511, D46).
pub fn signal_tokens(signals: &[String]) -> std::collections::BTreeSet<String> {
    normalize_signals(signals)
        .iter()
        .flat_map(|s| {
            s.split(|c: char| !c.is_alphanumeric())
                .filter(|t| t.chars().count() >= 4)
                .filter(|t| !COMMON.contains(t))
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// How many distinguishing tokens two signal sets share.
pub fn signal_overlap(a: &[String], b: &[String]) -> usize {
    let left = signal_tokens(a);
    signal_tokens(b)
        .iter()
        .filter(|t| left.contains(*t))
        .count()
}

/// One recorded application, reduced to what trust reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationFacts {
    /// Which project applied it. Trust counts **distinct** projects.
    pub project_id: uuid::Uuid,
    pub outcome: PatternOutcome,
    pub discovery: PatternDiscovery,
    /// Deterministic evidence collected in the applying project.
    pub has_evidence: bool,
    /// True when the applying project is where the pattern came from.
    pub is_origin: bool,
}

/// What the surfaces report, and what trust is derived from.
///
/// Never presented as a number of independent verifications, anywhere (FR-406).
/// The rendering that is allowed lives in [`PatternCounters::render`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PatternCounters {
    pub applications: usize,
    pub distinct_projects_applied: usize,
    pub qualifying_successes: usize,
    pub counterexamples: usize,
}

impl PatternCounters {
    /// The one permitted phrasing (`contracts/patterns.md` §Applications).
    pub fn render(&self) -> String {
        format!(
            "applications {} · distinct projects {} · independently validated in {} · counterexamples {}",
            self.applications,
            self.distinct_projects_applied,
            self.qualifying_successes,
            self.counterexamples
        )
    }
}

/// Count what the applications say.
///
/// Three things deliberately do not advance trust, and each is a filter here
/// rather than a rule someone must remember:
///
/// 1. **Repetition.** Ten applications in one project are one row upstream —
///    the unique key `(pattern_id, project_id, signal_digest)` sees to that —
///    and successes are counted by distinct project regardless.
/// 2. **The origin project.** `is_origin` is excluded: a pattern cannot
///    validate itself where it came from.
/// 3. **Cairn's own suggestion, unaided.** `cairn_suggested` with no evidence
///    counts as an application and not as a validation. An agent reading
///    Cairn's suggestion and agreeing with it is Cairn confirming Cairn
///    (FR-403).
pub fn count_applications(applications: &[ApplicationFacts]) -> PatternCounters {
    let distinct: std::collections::BTreeSet<uuid::Uuid> =
        applications.iter().map(|a| a.project_id).collect();

    let qualifying: std::collections::BTreeSet<uuid::Uuid> = applications
        .iter()
        .filter(|a| a.outcome == PatternOutcome::Resolved)
        .filter(|a| !a.is_origin)
        .filter(|a| a.discovery == PatternDiscovery::Independent || a.has_evidence)
        .map(|a| a.project_id)
        .collect();

    PatternCounters {
        applications: applications.len(),
        distinct_projects_applied: distinct.len(),
        qualifying_successes: qualifying.len(),
        counterexamples: applications
            .iter()
            .filter(|a| {
                matches!(
                    a.outcome,
                    PatternOutcome::NotApplicable | PatternOutcome::Failed
                )
            })
            .count(),
    }
}

/// The staged ladder (`contracts/patterns.md` §Applications).
///
/// `contested` is evaluated **before** `validated`, so a pattern carrying both
/// successes and counterexamples reports `contested` and both sides are stated.
/// The other order would let a pattern that failed somewhere present itself as
/// settled because it also succeeded somewhere (FR-405).
///
/// Nothing here deletes or demotes below the evidence: a contested pattern is
/// retained with its successes intact.
pub fn derive_pattern_trust(gate_passed: bool, counters: PatternCounters) -> PatternTrust {
    if !gate_passed {
        return PatternTrust::Candidate;
    }
    if counters.counterexamples > 0 {
        return PatternTrust::Contested;
    }
    if counters.qualifying_successes >= 1 {
        return PatternTrust::Validated;
    }
    PatternTrust::Sanitized
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn app(project: Uuid, outcome: PatternOutcome) -> ApplicationFacts {
        ApplicationFacts {
            project_id: project,
            outcome,
            discovery: PatternDiscovery::Independent,
            has_evidence: true,
            is_origin: false,
        }
    }

    #[test]
    fn signals_normalize_to_a_comparable_set() {
        let signals = vec![
            "  Could NOT find an available non-overlapping IPv4 address pool.  ".to_string(),
            "could not find an available non-overlapping ipv4 address pool".to_string(),
            "Docker bridge network create failure".to_string(),
        ];
        let normalized = normalize_signals(&signals);
        assert_eq!(
            normalized.len(),
            2,
            "case and punctuation are not two different signals: {normalized:?}"
        );
        assert!(normalized.windows(2).all(|w| w[0] <= w[1]), "sorted");
    }

    #[test]
    fn the_digest_does_not_depend_on_the_order_they_were_listed_in() {
        let a = vec!["beta signal".to_string(), "alpha signal".to_string()];
        let b = vec!["alpha signal".to_string(), "beta signal".to_string()];
        assert_eq!(signal_digest(&a), signal_digest(&b));
    }

    #[test]
    fn an_unrepresentable_signal_is_dropped_rather_than_counted() {
        let signals = vec!["real signal".to_string(), "   ".to_string()];
        assert_eq!(normalize_signals(&signals), vec!["real signal".to_string()]);
    }

    #[test]
    fn signals_are_bounded() {
        let many: Vec<String> = (0..40).map(|i| format!("signal number {i}")).collect();
        assert_eq!(normalize_signals(&many).len(), SIGNALS_MAX);
        let long = vec!["x".repeat(500)];
        assert_eq!(
            normalize_signals(&long)[0].chars().count(),
            SIGNAL_MAX_CHARS
        );
    }

    #[test]
    fn overlap_counts_distinguishing_tokens() {
        // What a pattern was written with, and what an error actually said.
        // They are not the same string and never will be.
        let pattern = vec![
            "could not find an available non-overlapping ipv4 address pool".to_string(),
            "docker bridge network create failure".to_string(),
        ];
        let observed = vec!["Error response from daemon: could not find an available, \
             non-overlapping IPv4 address pool among the defaults"
            .to_string()];
        assert!(
            signal_overlap(&pattern, &observed) >= 2,
            "a real error message must be able to match the signature written for it"
        );

        // And an unrelated failure does not.
        let unrelated = vec!["connection refused connecting to the metrics endpoint".to_string()];
        assert_eq!(signal_overlap(&pattern, &unrelated), 0);
    }

    #[test]
    fn common_words_alone_do_not_make_a_match() {
        // Two unrelated messages sharing only the words every error contains.
        let a = vec!["could not open the file that was requested".to_string()];
        let b = vec!["could not parse the value that was given".to_string()];
        assert!(
            signal_overlap(&a, &b) < 2,
            "generic words must not reach the two-token minimum on their own: {}",
            signal_overlap(&a, &b)
        );
    }

    #[test]
    fn a_token_is_matched_exactly_or_not_at_all() {
        // No stemming and no distance. `pools` is not `pool`.
        let a = vec!["address pools exhausted".to_string()];
        let b = vec!["address pool exhausted".to_string()];
        let shared = signal_tokens(&a).intersection(&signal_tokens(&b)).count();
        assert_eq!(
            shared, 2,
            "`address` and `exhausted` match; `pools` and `pool` are different tokens"
        );
    }

    #[test]
    fn ten_applications_in_one_project_count_once() {
        let one = Uuid::now_v7();
        let apps: Vec<ApplicationFacts> = (0..10)
            .map(|_| app(one, PatternOutcome::Resolved))
            .collect();
        let counters = count_applications(&apps);
        assert_eq!(counters.distinct_projects_applied, 1);
        assert_eq!(
            counters.qualifying_successes, 1,
            "repetition in one project is one success, not ten"
        );
    }

    #[test]
    fn the_origin_project_cannot_validate_its_own_pattern() {
        let apps = vec![ApplicationFacts {
            is_origin: true,
            ..app(Uuid::now_v7(), PatternOutcome::Resolved)
        }];
        let counters = count_applications(&apps);
        assert_eq!(counters.applications, 1);
        assert_eq!(counters.qualifying_successes, 0);
        assert_eq!(
            derive_pattern_trust(true, counters),
            PatternTrust::Sanitized
        );
    }

    #[test]
    fn a_suggestion_agreed_with_is_not_a_validation() {
        let unaided = vec![ApplicationFacts {
            discovery: PatternDiscovery::CairnSuggested,
            has_evidence: false,
            ..app(Uuid::now_v7(), PatternOutcome::Resolved)
        }];
        assert_eq!(count_applications(&unaided).qualifying_successes, 0);

        // The same suggestion, confirmed by evidence collected in the applying
        // project, does validate.
        let evidenced = vec![ApplicationFacts {
            discovery: PatternDiscovery::CairnSuggested,
            has_evidence: true,
            ..app(Uuid::now_v7(), PatternOutcome::Resolved)
        }];
        assert_eq!(count_applications(&evidenced).qualifying_successes, 1);
    }

    #[test]
    fn contested_is_decided_before_validated() {
        let apps = vec![
            app(Uuid::now_v7(), PatternOutcome::Resolved),
            app(Uuid::now_v7(), PatternOutcome::NotApplicable),
        ];
        let counters = count_applications(&apps);
        assert_eq!(counters.qualifying_successes, 1);
        assert_eq!(counters.counterexamples, 1);
        assert_eq!(
            derive_pattern_trust(true, counters),
            PatternTrust::Contested,
            "a pattern with a counterexample must not present itself as settled"
        );
    }

    #[test]
    fn a_failed_outcome_is_a_counterexample_too() {
        let apps = vec![app(Uuid::now_v7(), PatternOutcome::Failed)];
        assert_eq!(count_applications(&apps).counterexamples, 1);
    }

    #[test]
    fn an_unpromoted_candidate_is_never_trusted() {
        let apps = vec![app(Uuid::now_v7(), PatternOutcome::Resolved)];
        assert_eq!(
            derive_pattern_trust(false, count_applications(&apps)),
            PatternTrust::Candidate
        );
    }

    #[test]
    fn the_rendering_never_says_verifications() {
        let counters = PatternCounters {
            applications: 12,
            distinct_projects_applied: 3,
            qualifying_successes: 1,
            counterexamples: 2,
        };
        let rendered = counters.render();
        assert_eq!(
            rendered,
            "applications 12 · distinct projects 3 · independently validated in 1 · counterexamples 2"
        );
        assert!(
            !rendered.to_lowercase().contains("verif"),
            "no count is ever presented as a number of verifications: {rendered}"
        );
    }
}
