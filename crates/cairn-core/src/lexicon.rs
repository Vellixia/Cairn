//! Turning transient vendor text into a semantic signal, deterministically
//! (`contracts/extraction.md` §13.7).
//!
//! ## What this is for
//!
//! Feature 005 exists to learn decisions, constraints and procedures — not only
//! that a test went from red to green. The material that expresses a decision
//! is prose, and prose does not cross the machine boundary. So the local
//! machine reads the prose **in memory**, during the hook invocation that
//! already parses and redacts it, and emits four values: a kind from a closed
//! enumeration, two tokens the session's own event stream already established,
//! and the ordinal of the event that established them. Nothing else survives.
//!
//! ## Why a model is not involved
//!
//! Every step here is a table lookup, a set intersection or a total order. A
//! local model *may* propose tokens instead of step 3, but step 4's vocabulary
//! check governs either way, so the model would be an optimization and never
//! the gate. Determinism is what makes the result reproducible by the server,
//! which re-derives the vocabulary independently and refuses a token it cannot
//! justify (`contracts/safe-events.md` §7.1 step 7).
//!
//! ## Why declining is the common case, and correct
//!
//! A recorded decision Cairn cannot ground in its own event stream is a claim
//! it cannot explain later, and an unexplainable claim in durable memory is
//! worse than an absent one. Every decline path below names *which* condition
//! applied, because a single reason would make the decline rate
//! uninterpretable — an implementer could not tell a lexicon that never matches
//! from a vocabulary that is too thin.

use crate::event::{
    DecisionKind, DeclineReason, EventContent, EventKind, InstructionKind, VocabToken,
};
use crate::knowledge::normalize_value_key;
use crate::redact::redact;
use crate::vocabulary::{SessionVocabulary, VocabEntry};
use std::collections::BTreeMap;

/// The version of the marker table below.
///
/// It travels on every signal because only a server that recognises the version
/// can reproduce the classification. A server that does not know a version
/// rejects the event at ingest rather than storing a claim it cannot explain.
pub const LEXICON_VERSION: u16 = 1;

/// The longest marker phrase in words, so matching knows how far to look ahead.
const MAX_MARKER_WORDS: usize = 3;

/// Which vendor field the material came from.
///
/// The event kind follows from this and from nothing else — not from any
/// reading of the text (`contracts/extraction.md` §13.7 step 4a). An earlier
/// design resolved it by grammatical person, which is not a deterministic rule
/// and was undefined for every input that is not a clean imperative.
///
/// It is also the right semantics: an instruction is something the *user* said,
/// and a decision is something the *session* concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceRole {
    /// The vendor's user-prompt field. Emits `user_instruction_signal`.
    UserPrompt,
    /// The vendor's settled assistant-message field. Emits `decision_signal`.
    AssistantMessage,
}

impl SourceRole {
    pub fn event_kind(self) -> EventKind {
        match self {
            SourceRole::UserPrompt => EventKind::UserInstructionSignal,
            SourceRole::AssistantMessage => EventKind::DecisionSignal,
        }
    }
}

/// One row of the closed marker table.
///
/// Two columns, and a marker may be absent from either. The columns
/// **constrain** rather than choose: the source role picks the column, and a
/// marker with no entry in the chosen column declines. So `adopt` from a user
/// prompt declines, and `scope to` from an assistant message declines.
struct Marker {
    words: &'static [&'static str],
    decision: Option<DecisionKind>,
    instruction: Option<InstructionKind>,
}

/// The literal table from `contracts/extraction.md` §13.7 step 2.
///
/// Order here is irrelevant: matching is longest-phrase-wins, resolved by
/// length rather than by position, so no row can shadow another by being
/// written first.
const MARKERS: &[Marker] = &[
    m(&["use"], Some(DecisionKind::Adopt), None),
    m(&["switch", "to"], Some(DecisionKind::Adopt), None),
    m(&["go", "with"], Some(DecisionKind::Adopt), None),
    m(&["adopt"], Some(DecisionKind::Adopt), None),
    m(&["don't", "use"], Some(DecisionKind::Reject), None),
    m(&["drop"], Some(DecisionKind::Reject), None),
    m(&["stop", "using"], Some(DecisionKind::Reject), None),
    m(&["reject"], Some(DecisionKind::Reject), None),
    m(&["later"], Some(DecisionKind::Defer), None),
    m(&["defer"], Some(DecisionKind::Defer), None),
    m(&["not", "now"], Some(DecisionKind::Defer), None),
    m(&["postpone"], Some(DecisionKind::Defer), None),
    m(
        &["must"],
        Some(DecisionKind::Constrain),
        Some(InstructionKind::Require),
    ),
    m(
        &["always"],
        Some(DecisionKind::Constrain),
        Some(InstructionKind::Require),
    ),
    m(
        &["require"],
        Some(DecisionKind::Constrain),
        Some(InstructionKind::Require),
    ),
    m(
        &["enforce"],
        Some(DecisionKind::Constrain),
        Some(InstructionKind::Require),
    ),
    m(
        &["never"],
        Some(DecisionKind::Revert),
        Some(InstructionKind::Forbid),
    ),
    m(
        &["forbid"],
        Some(DecisionKind::Revert),
        Some(InstructionKind::Forbid),
    ),
    m(
        &["don't"],
        Some(DecisionKind::Revert),
        Some(InstructionKind::Forbid),
    ),
    m(
        &["prefer"],
        Some(DecisionKind::Prefer),
        Some(InstructionKind::Prefer),
    ),
    m(
        &["rather", "than"],
        Some(DecisionKind::Prefer),
        Some(InstructionKind::Prefer),
    ),
    m(
        &["instead", "of"],
        Some(DecisionKind::Prefer),
        Some(InstructionKind::Prefer),
    ),
    m(&["revert"], Some(DecisionKind::Revert), None),
    m(&["undo"], Some(DecisionKind::Revert), None),
    m(&["roll", "back"], Some(DecisionKind::Revert), None),
    m(&["only", "in"], None, Some(InstructionKind::Scope)),
    m(&["scope", "to"], None, Some(InstructionKind::Scope)),
    m(&["limit", "to"], None, Some(InstructionKind::Scope)),
    m(&["actually"], None, Some(InstructionKind::Correct)),
    m(&["no", "—"], None, Some(InstructionKind::Correct)),
    m(&["correction"], None, Some(InstructionKind::Correct)),
];

const fn m(
    words: &'static [&'static str],
    decision: Option<DecisionKind>,
    instruction: Option<InstructionKind>,
) -> Marker {
    Marker {
        words,
        decision,
        instruction,
    }
}

/// The kind a marker resolves to once the source role has chosen its column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkerKind {
    Decision(DecisionKind),
    Instruction(InstructionKind),
}

/// The four values a successful mapping produces, plus the kind that carries
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSignal {
    pub kind: EventKind,
    pub content: EventContent,
}

/// Map transient vendor text to a semantic signal, or say why it cannot be.
///
/// `established_values` maps a normalized `topic_key` to the normalized
/// `value_key` this project has already established for it. It supplies the
/// step-5 fallback object for `adopt` and `reject`, and an empty map simply
/// means the fallback is unavailable — which declines rather than inventing an
/// object.
///
/// The text is redacted **first**, and every later step reads only the redacted
/// form, so a secret can influence neither the classification nor the token
/// selection. This ordering is why a credential in a command line can never
/// become a token that legitimises itself.
pub fn map_semantic_signal(
    text: &str,
    role: SourceRole,
    vocabulary: &SessionVocabulary,
    established_values: &BTreeMap<String, String>,
) -> Result<SemanticSignal, DeclineReason> {
    // Step 1 — redact.
    let redacted = redact(text);
    let words = tokenize(&redacted);

    // Step 2 + 4a — classify from the fixed lexicon, in the column the source
    // role selects.
    let kind = classify(&words, role)?;

    // Steps 3 and 4 — candidates, then intersection with the vocabulary. This
    // is the step that makes the whole design safe: every surviving token is
    // something the event stream already established, so no word that is merely
    // *in the prose* can survive.
    let survivors = surviving_tokens(&words, vocabulary);

    // Step 5 — assign roles by the fixed total rank.
    let (subject, subject_entry) = match survivors.first() {
        Some(first) => first.clone(),
        None => return Err(DeclineReason::InsufficientVocabulary),
    };
    let (object, object_entry) = match survivors.iter().skip(1).find(|(t, _)| *t != subject) {
        Some((token, entry)) => (token.clone(), Some(*entry)),
        // The closed enumeration may supply the object for `adopt` and
        // `reject`: the subject's established value key is a value this project
        // already holds for this subject, so naming it invents nothing.
        None => match fallback_object(kind, &subject, established_values) {
            Some(token) => (token, None),
            None => return Err(DeclineReason::InsufficientVocabulary),
        },
    };

    // A claim about nothing. Two names for one thing is not a subject and an
    // object; it is one token written twice.
    if subject == object {
        return Err(DeclineReason::InsufficientVocabulary);
    }

    let subject_token =
        VocabToken::subject(&subject).map_err(|_| DeclineReason::InsufficientVocabulary)?;
    let object_token =
        VocabToken::object(&object).map_err(|_| DeclineReason::InsufficientVocabulary)?;

    // The highest ordinal among the events justifying the two tokens, so a
    // server refusal can name what was missing. A token an established project
    // key supplied has no ordinal and contributes none.
    let justified_by_seq = [subject_entry.seq, object_entry.and_then(|e| e.seq)]
        .into_iter()
        .flatten()
        .max();

    let content = match kind {
        MarkerKind::Decision(decision_kind) => EventContent::Decision {
            decision_kind,
            subject_token,
            object_token,
            justified_by_seq,
            lexicon_version: LEXICON_VERSION,
        },
        MarkerKind::Instruction(instruction_kind) => EventContent::Instruction {
            instruction_kind,
            subject_token,
            object_token,
            justified_by_seq,
            lexicon_version: LEXICON_VERSION,
        },
    };

    Ok(SemanticSignal {
        kind: role.event_kind(),
        content,
    })
}

/// Step 2 and step 4a, together, because neither is decidable alone.
///
/// Matching is longest-phrase-wins and a longer match consumes its span, so
/// `don't use X` matches `don't use` and the shorter `don't` never competes for
/// the same span. Without that rule the commonest phrasing in the table would
/// decline itself.
fn classify(words: &[String], role: SourceRole) -> Result<MarkerKind, DeclineReason> {
    let mut found: Vec<MarkerKind> = Vec::new();
    let mut i = 0;
    while i < words.len() {
        match longest_marker_at(words, i) {
            Some((marker, len)) => {
                // The columns constrain rather than choose. A marker with no
                // counterpart for this source role is not a weaker signal; it
                // is evidence the text is not the thing the role expects.
                let resolved = match role {
                    SourceRole::AssistantMessage => marker.decision.map(MarkerKind::Decision),
                    SourceRole::UserPrompt => marker.instruction.map(MarkerKind::Instruction),
                };
                let Some(resolved) = resolved else {
                    return Err(DeclineReason::AmbiguousClassification);
                };
                found.push(resolved);
                i += len;
            }
            None => i += 1,
        }
    }

    let mut distinct = found.clone();
    distinct.dedup();
    match (found.first(), distinct.len()) {
        (None, _) => Err(DeclineReason::NoSafeSemanticMapping),
        (Some(first), 1) => Ok(*first),
        // An ambiguous instruction is not a fact about the project, and
        // guessing between two readings would fabricate one.
        (Some(_), _) => Err(DeclineReason::AmbiguousClassification),
    }
}

fn longest_marker_at(words: &[String], at: usize) -> Option<(&'static Marker, usize)> {
    for len in (1..=MAX_MARKER_WORDS.min(words.len() - at)).rev() {
        let span = &words[at..at + len];
        if let Some(marker) = MARKERS
            .iter()
            .find(|m| m.words.len() == len && m.words.iter().zip(span).all(|(a, b)| *a == b))
        {
            return Some((marker, len));
        }
    }
    None
}

/// Steps 3 and 4: unigrams and adjacent bigrams, normalized, intersected with
/// the vocabulary, and ordered by the fixed total rank of §13.5.
///
/// The order is total so the result cannot depend on iteration order: rank
/// first, then the lowest justifying ordinal (a token an established key
/// supplied has none and sorts last), then lexicographically.
fn surviving_tokens(words: &[String], vocabulary: &SessionVocabulary) -> Vec<(String, VocabEntry)> {
    let mut candidates: Vec<String> = Vec::new();
    for (i, word) in words.iter().enumerate() {
        if let Some(one) = normalize_value_key(word) {
            candidates.push(one);
        }
        if let Some(next) = words.get(i + 1) {
            if let Some(two) = normalize_value_key(&format!("{word} {next}")) {
                candidates.push(two);
            }
        }
    }

    let mut survivors: Vec<(String, VocabEntry)> = Vec::new();
    for candidate in candidates {
        if survivors.iter().any(|(t, _)| *t == candidate) {
            continue;
        }
        if let Some(entry) = vocabulary.entry(&candidate) {
            survivors.push((candidate, entry));
        }
    }

    survivors.sort_by(|(a_token, a), (b_token, b)| {
        b.rank
            .cmp(&a.rank)
            .then_with(|| a.seq.unwrap_or(u64::MAX).cmp(&b.seq.unwrap_or(u64::MAX)))
            .then_with(|| a_token.cmp(b_token))
    });
    survivors
}

/// The closed enumeration's object, available to `adopt` and `reject` only.
///
/// Restricted to those two because they are the kinds whose object is a value
/// the project already holds for the subject. For `defer` or `correct` there is
/// no such value, and supplying one would state something the text did not.
fn fallback_object(
    kind: MarkerKind,
    subject: &str,
    established_values: &BTreeMap<String, String>,
) -> Option<String> {
    match kind {
        MarkerKind::Decision(DecisionKind::Adopt) | MarkerKind::Decision(DecisionKind::Reject) => {
            established_values.get(subject).cloned()
        }
        _ => None,
    }
}

/// Case-fold and split into matchable words.
///
/// A word is a run of letters, digits and apostrophes, so `don't` stays one
/// word and `roll back` stays two. Dashes long enough to be punctuation are
/// emitted as their own token, because the table contains one (`no —`) and a
/// separator that vanished would make that row unmatchable.
fn tokenize(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for raw in text.chars() {
        // A typographic apostrophe and a typewriter one are the same character
        // for this purpose; a text that used the first would otherwise never
        // match `don't`.
        let c = match raw {
            '\u{2019}' => '\'',
            other => other,
        };
        if c.is_alphanumeric() || c == '\'' {
            current.extend(c.to_lowercase());
            continue;
        }
        if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        if matches!(c, '—' | '–') {
            words.push(c.to_string());
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ChangeKind, FileIdentity};

    fn file(path: &str) -> EventContent {
        EventContent::File {
            repo_file: Some(path.to_string()),
            repo_file_from: None,
            change_kind: Some(ChangeKind::Modified),
            file_identity: FileIdentity::Present,
        }
    }

    fn command(line: &str) -> EventContent {
        EventContent::Command {
            command_line: line.to_string(),
            exit_status: Some(0),
        }
    }

    /// A session that has touched a storage module and run a deploy command.
    fn session() -> SessionVocabulary {
        let mut v = SessionVocabulary::new();
        v.observe_at(
            Some(1),
            EventKind::FileChanged,
            Some(&file("storage/postgresql.rs")),
        );
        v.observe_at(
            Some(2),
            EventKind::CommandExecuted,
            Some(&command("deploy stack")),
        );
        v
    }

    fn no_values() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn map(text: &str, role: SourceRole) -> Result<SemanticSignal, DeclineReason> {
        map_semantic_signal(text, role, &session(), &no_values())
    }

    #[test]
    fn an_assistant_turn_naming_two_established_tokens_becomes_a_decision() {
        let signal = map(
            "we should use postgresql for storage",
            SourceRole::AssistantMessage,
        )
        .expect("a decision");
        assert_eq!(signal.kind, EventKind::DecisionSignal);
        match signal.content {
            EventContent::Decision {
                decision_kind,
                subject_token,
                object_token,
                lexicon_version,
                ..
            } => {
                assert_eq!(decision_kind, DecisionKind::Adopt);
                // `storage` is a directory component and outranks `postgresql`,
                // which is a file stem, so the subject is the coarser subject.
                assert_eq!(subject_token.as_str(), "storage");
                assert_eq!(object_token.as_str(), "postgresql");
                assert_eq!(lexicon_version, LEXICON_VERSION);
            }
            other => panic!("expected a decision, got {other:?}"),
        }
    }

    #[test]
    fn a_prompt_sentence_carries_nothing_that_is_not_already_in_the_vocabulary() {
        // The property the whole design exists for. Every word here is prose,
        // so no token survives step 4 and there is nothing to record.
        let refusal = map(
            "please always remember that the api key is sk-abc123def456",
            SourceRole::UserPrompt,
        )
        .expect_err("a sentence must not become a claim");
        assert_eq!(refusal, DeclineReason::InsufficientVocabulary);
    }

    #[test]
    fn a_credential_cannot_reach_classification_or_token_selection() {
        // Redaction runs first, so by the time anything reads the text the
        // credential is gone. Even if it survived, it is not in the vocabulary.
        let refusal = map(
            "use ghp_abcdefghijklmnopqrstuvwxyz0123456789 always",
            SourceRole::AssistantMessage,
        )
        .expect_err("a credential is not a subject");
        // Two markers of different kinds, so this declines before tokens even
        // matter — and `AmbiguousClassification` is the honest reason.
        assert_eq!(refusal, DeclineReason::AmbiguousClassification);
    }

    #[test]
    fn no_marker_at_all_is_its_own_reason() {
        let refusal = map("storage and postgresql", SourceRole::AssistantMessage)
            .expect_err("nothing indicated a decision");
        assert_eq!(refusal, DeclineReason::NoSafeSemanticMapping);
    }

    #[test]
    fn the_longest_marker_wins_its_span_so_the_commonest_phrasing_does_not_decline_itself() {
        // `don't use` must match as `reject`. If the shorter `don't` competed
        // for the same span the two would read as different kinds and the most
        // ordinary phrasing in the table would refuse itself.
        let signal = map(
            "don't use postgresql for storage",
            SourceRole::AssistantMessage,
        )
        .expect("a rejection");
        match signal.content {
            EventContent::Decision { decision_kind, .. } => {
                assert_eq!(decision_kind, DecisionKind::Reject);
            }
            other => panic!("expected a decision, got {other:?}"),
        }
    }

    #[test]
    fn the_source_role_chooses_the_kind_and_a_marker_without_a_counterpart_declines() {
        // `adopt` is decision-only. From a user prompt it has no counterpart,
        // and guessing an instruction kind for it would fabricate one.
        let refusal = map("adopt postgresql for storage", SourceRole::UserPrompt)
            .expect_err("a decision-only marker in a prompt");
        assert_eq!(refusal, DeclineReason::AmbiguousClassification);

        // `scope to` is instruction-only, so the mirror case also declines.
        let refusal = map(
            "scope to storage and postgresql",
            SourceRole::AssistantMessage,
        )
        .expect_err("an instruction-only marker in an assistant turn");
        assert_eq!(refusal, DeclineReason::AmbiguousClassification);
    }

    #[test]
    fn a_marker_present_in_both_columns_maps_by_source_role_with_no_ambiguity_left() {
        let instruction = map(
            "always use... prefer postgresql over storage",
            SourceRole::UserPrompt,
        );
        // `prefer` exists in both columns; from a prompt it is an instruction.
        // (`use` is decision-only, so this particular text declines — the
        // assertion that matters is that it declines for the stated reason and
        // not by guessing.)
        assert_eq!(
            instruction.unwrap_err(),
            DeclineReason::AmbiguousClassification
        );

        let signal =
            map("prefer postgresql for storage", SourceRole::UserPrompt).expect("an instruction");
        assert_eq!(signal.kind, EventKind::UserInstructionSignal);
        match signal.content {
            EventContent::Instruction {
                instruction_kind, ..
            } => assert_eq!(instruction_kind, InstructionKind::Prefer),
            other => panic!("expected an instruction, got {other:?}"),
        }
    }

    #[test]
    fn two_markers_of_different_kinds_decline_rather_than_guessing() {
        let refusal = map(
            "use postgresql, and never storage",
            SourceRole::AssistantMessage,
        )
        .expect_err("an ambiguous turn is not a fact about the project");
        assert_eq!(refusal, DeclineReason::AmbiguousClassification);
    }

    #[test]
    fn one_surviving_token_is_a_claim_about_nothing() {
        let refusal = map("use storage", SourceRole::AssistantMessage)
            .expect_err("one token is a subject with no object");
        assert_eq!(refusal, DeclineReason::InsufficientVocabulary);
    }

    #[test]
    fn an_established_value_key_may_supply_the_object_for_adopt_and_reject() {
        let mut established = BTreeMap::new();
        established.insert("storage".to_string(), "postgresql_16".to_string());
        let signal = map_semantic_signal(
            "use storage",
            SourceRole::AssistantMessage,
            &session(),
            &established,
        )
        .expect("the closed enumeration supplied the object");
        match signal.content {
            EventContent::Decision {
                subject_token,
                object_token,
                ..
            } => {
                assert_eq!(subject_token.as_str(), "storage");
                assert_eq!(object_token.as_str(), "postgresql_16");
            }
            other => panic!("expected a decision, got {other:?}"),
        }

        // The fallback is confined to `adopt` and `reject`. `defer` has no
        // value the project already holds for the subject, so supplying one
        // would state something the text did not.
        let refusal = map_semantic_signal(
            "postpone storage",
            SourceRole::AssistantMessage,
            &session(),
            &established,
        )
        .expect_err("defer has no fallback object");
        assert_eq!(refusal, DeclineReason::InsufficientVocabulary);
    }

    #[test]
    fn the_result_does_not_depend_on_the_order_the_vocabulary_was_built_in() {
        let mut forwards = SessionVocabulary::new();
        forwards.observe_at(
            Some(1),
            EventKind::FileChanged,
            Some(&file("storage/postgresql.rs")),
        );
        forwards.observe_at(
            Some(2),
            EventKind::CommandExecuted,
            Some(&command("deploy stack")),
        );

        let mut backwards = SessionVocabulary::new();
        backwards.observe_at(
            Some(2),
            EventKind::CommandExecuted,
            Some(&command("deploy stack")),
        );
        backwards.observe_at(
            Some(1),
            EventKind::FileChanged,
            Some(&file("storage/postgresql.rs")),
        );

        let text = "use postgresql for storage";
        assert_eq!(
            map_semantic_signal(text, SourceRole::AssistantMessage, &forwards, &no_values()),
            map_semantic_signal(text, SourceRole::AssistantMessage, &backwards, &no_values()),
        );
    }

    #[test]
    fn the_ordinal_recorded_is_the_highest_among_the_events_that_justified_the_tokens() {
        let signal =
            map("use postgresql for storage", SourceRole::AssistantMessage).expect("a decision");
        match signal.content {
            // Both tokens came from the file event at seq 1, so that is the
            // ordinal a server refusal would name.
            EventContent::Decision {
                justified_by_seq, ..
            } => assert_eq!(justified_by_seq, Some(1)),
            other => panic!("expected a decision, got {other:?}"),
        }
    }

    #[test]
    fn a_bigram_is_a_candidate_so_a_two_word_subject_can_survive() {
        let v = SessionVocabulary::new().with_established_keys(["storage_authority"]);
        let mut established = BTreeMap::new();
        established.insert("storage_authority".to_string(), "postgresql".to_string());
        let signal = map_semantic_signal(
            "use storage authority",
            SourceRole::AssistantMessage,
            &v,
            &established,
        )
        .expect("a decision about a two-word subject");
        match signal.content {
            EventContent::Decision { subject_token, .. } => {
                assert_eq!(subject_token.as_str(), "storage_authority");
            }
            other => panic!("expected a decision, got {other:?}"),
        }
    }

    #[test]
    fn a_typographic_apostrophe_matches_the_same_marker_as_a_typewriter_one() {
        let curly = map(
            "don\u{2019}t use postgresql for storage",
            SourceRole::AssistantMessage,
        )
        .expect("a rejection");
        let straight = map(
            "don't use postgresql for storage",
            SourceRole::AssistantMessage,
        )
        .expect("a rejection");
        assert_eq!(curly, straight);
    }
}
