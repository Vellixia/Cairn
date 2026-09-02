//! The session vocabulary a semantic signal must justify its tokens against
//! (`contracts/extraction.md` §13.3).
//!
//! ## Why this exists at all
//!
//! Feature 005 has to learn what was *decided*, not only that a test went from
//! red to green. The first design gave `decision_signal` and
//! `user_instruction_signal` no content, on the grounds that any text derived
//! from a prompt is a transcript fragment. That destroyed the information at
//! the machine boundary, where nothing downstream can recover it, and left the
//! design unable to do the thing it is for.
//!
//! A length-capped text field is not the repair. Redaction is pattern-based and
//! describes itself as a mechanism rather than a guarantee, and a bounded
//! free-text field derived from a prompt is still a prompt fragment.
//! Constitution V prefers structural prevention: a record with no column for a
//! secret cannot carry one, and a capped text column is a procedural rule
//! wearing a number.
//!
//! Nor is a charset enough. `the api key is sk-abc123` normalizes to
//! `the_api_key_is_sk_abc123`, which is inside any charset and any length.
//! Shape alone constrains nothing.
//!
//! So a token must be **justified against evidence already in the event
//! stream**. A prompt sentence cannot survive, because its words are not file
//! segments, command verbs, test identifiers or established project keys — and
//! neither is a credential.
//!
//! ## One implementation, both sides
//!
//! The client checks a token before constructing the event; the server checks
//! it independently against the events it already holds. Two implementations
//! would be two things that can drift, and drift here means the client
//! constructs signals the server permanently refuses — destroying decisions
//! silently. So the derivation lives here, once, and both call it.

use crate::event::{EventContent, EventKind, VocabToken};
use crate::knowledge::normalize_topic_key;
use std::collections::BTreeSet;

/// One session's derived vocabulary.
///
/// A set, deliberately: membership is the only question anyone asks of it, and
/// exposing an order would invite something to depend on one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionVocabulary {
    tokens: BTreeSet<String>,
}

impl SessionVocabulary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed with the topic and value keys this project's knowledge already
    /// establishes.
    ///
    /// Both sources are required. A server that justified tokens only from
    /// session events would refuse tokens the client legitimately justified
    /// from established project keys — and the refusal is permanent, so the
    /// decision is destroyed rather than deferred
    /// (`contracts/safe-events.md` §7.1 step 7).
    pub fn with_established_keys<'a>(mut self, keys: impl IntoIterator<Item = &'a str>) -> Self {
        for key in keys {
            // An established key may be dot-segmented. Both the whole key and
            // each segment count: a decision citing `deploy` is justified by an
            // established `deploy.images`, because the subject is visibly part
            // of this project's vocabulary either way.
            if let Some(normalized) = normalize_topic_key(key) {
                for segment in normalized.split('.') {
                    self.insert(segment);
                }
                self.insert(&normalized);
            }
        }
        self
    }

    /// Add everything one event contributes.
    ///
    /// Only events with a **lower `session_seq`** than the signal being checked
    /// should be fed in. A token justified only by a later event is refused:
    /// the machine that built the signal knew the earlier events too, so it
    /// could not legitimately have cited a later one, and accepting it would
    /// make justification depend on delivery order.
    pub fn observe(&mut self, kind: EventKind, content: Option<&EventContent>) {
        let Some(content) = content else { return };
        match content {
            // Path segments, not file contents — Cairn never reads those. A
            // deliberately-named file can therefore only contribute a token
            // already visible in the repository to anyone who can read it.
            EventContent::File {
                repo_file,
                repo_file_from,
                ..
            } => {
                for path in [repo_file, repo_file_from].into_iter().flatten() {
                    self.add_path_segments(path);
                }
            }
            // The leading binary and its subcommand. Not the arguments: an
            // argument is where a value lives, and a value is not a verb.
            //
            // Redaction has already run on this string by the time it exists
            // (`contracts/extraction.md` §13.3), which is what stops a
            // credential entering the vocabulary and then legitimising a token
            // for itself.
            EventContent::Command { command_line, .. } => {
                self.add_leading_words(command_line, 2);
            }
            EventContent::TestInvocation { test_command } => {
                // A test command contributes more of itself than a shell
                // command does, because a suite identifier is often the third
                // or fourth word (`cargo test -p cairn-core`).
                self.add_leading_words(test_command, 4);
            }
            EventContent::Tool { vendor_tool, .. }
            | EventContent::ToolFailure { vendor_tool, .. } => {
                self.insert_normalized(vendor_tool);
            }
            _ => {}
        }
        let _ = kind;
    }

    /// Whether a token is justified.
    pub fn justifies(&self, token: &VocabToken) -> bool {
        self.contains(token.as_str())
    }

    pub fn contains(&self, token: &str) -> bool {
        // A dot-segmented token is justified when every one of its segments is.
        // `deploy.images` from a session that has seen `deploy/images.rs` is
        // the intended case; a token with an unjustified segment is not.
        token
            .split('.')
            .all(|segment| self.tokens.contains(segment))
            || self.tokens.contains(token)
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The tokens, sorted, for a diagnostic that needs to show its working.
    pub fn tokens(&self) -> impl Iterator<Item = &str> {
        self.tokens.iter().map(String::as_str)
    }

    fn add_path_segments(&mut self, path: &str) {
        for segment in path.split('/') {
            // A file's extension is not part of its identity here: `sync.rs`
            // and `sync.py` name the same module in two languages, and `rs` is
            // not a subject anyone decides about.
            let stem = segment.rsplit_once('.').map_or(segment, |(stem, _)| stem);
            self.insert_normalized(stem);
        }
    }

    fn add_leading_words(&mut self, text: &str, how_many: usize) {
        for word in text.split_whitespace().take(how_many) {
            // Flags are not verbs. `-p` and `--release` say how, not what.
            if word.starts_with('-') {
                continue;
            }
            // A path-shaped invocation contributes its final component:
            // `./scripts/deploy.sh` is `deploy`.
            let last = word.rsplit('/').next().unwrap_or(word);
            let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);
            self.insert_normalized(stem);
        }
    }

    fn insert_normalized(&mut self, raw: &str) {
        if let Some(normalized) = normalize_topic_key(raw) {
            for segment in normalized.split('.') {
                self.insert(segment);
            }
        }
    }

    fn insert(&mut self, token: &str) {
        if !token.is_empty() {
            self.tokens.insert(token.to_string());
        }
    }
}

/// Build a vocabulary from a session's accepted events and a project's keys.
///
/// `events` must already be filtered to a **lower `session_seq`** than the
/// signal under test and ordered by it. Ordering is the caller's job because
/// only the caller knows where the events came from — a spool drain, a batch,
/// or a database read — and silently sorting here would hide a caller that had
/// them out of order for a reason worth noticing.
pub fn derive<'a>(
    events: impl IntoIterator<Item = (EventKind, Option<&'a EventContent>)>,
    established_keys: impl IntoIterator<Item = &'a str>,
) -> SessionVocabulary {
    let mut vocabulary = SessionVocabulary::new().with_established_keys(established_keys);
    for (kind, content) in events {
        vocabulary.observe(kind, content);
    }
    vocabulary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{ChangeKind, FileIdentity, ToolClass};

    fn file(path: &str) -> EventContent {
        EventContent::File {
            repo_file: Some(path.to_string()),
            repo_file_from: None,
            change_kind: Some(ChangeKind::Modified),
            file_identity: FileIdentity::Present,
        }
    }

    #[test]
    fn a_files_path_segments_become_tokens_and_its_extension_does_not() {
        let mut v = SessionVocabulary::new();
        v.observe(
            EventKind::FileChanged,
            Some(&file("crates/cairnd/src/sync.rs")),
        );
        for expected in ["crates", "cairnd", "src", "sync"] {
            assert!(v.contains(expected), "{expected} is not in the vocabulary");
        }
        assert!(!v.contains("rs"), "a file extension is not a subject");
    }

    #[test]
    fn a_command_contributes_its_verb_and_subcommand_but_not_its_arguments() {
        let mut v = SessionVocabulary::new();
        v.observe(
            EventKind::CommandExecuted,
            Some(&EventContent::Command {
                command_line: "cargo build --release --target wasm32".into(),
                exit_status: Some(0),
            }),
        );
        assert!(v.contains("cargo"));
        assert!(v.contains("build"));
        // An argument is where a value lives, and a value is not a verb.
        assert!(!v.contains("wasm32"));
        assert!(!v.contains("release"));
    }

    #[test]
    fn a_prompt_sentence_justifies_nothing() {
        // The property the whole mechanism exists for. None of these words is a
        // file segment, a command verb, a test identifier or an established
        // key, so none of them can carry a decision across the boundary.
        let mut v = SessionVocabulary::new();
        v.observe(
            EventKind::FileChanged,
            Some(&file("crates/cairnd/src/sync.rs")),
        );
        for sentence_word in [
            "please",
            "the_api_key_is_sk_abc123",
            "remember",
            "important",
            "hunter2",
        ] {
            assert!(
                !v.contains(sentence_word),
                "{sentence_word:?} was justified by a vocabulary it has nothing to do with"
            );
        }
    }

    #[test]
    fn a_secret_in_a_command_cannot_legitimise_itself() {
        // Redaction runs before derivation, so by the time a command reaches
        // here the credential is gone. What this asserts is the second line of
        // defence: even unredacted, only the leading words contribute, so a
        // credential in an argument never enters the vocabulary.
        let mut v = SessionVocabulary::new();
        v.observe(
            EventKind::CommandExecuted,
            Some(&EventContent::Command {
                command_line: "deploy --token ghp_abcdefghijklmnop".into(),
                exit_status: None,
            }),
        );
        assert!(v.contains("deploy"));
        assert!(!v.contains("ghp_abcdefghijklmnop"));
        assert!(!v.contains("token"));
    }

    #[test]
    fn an_established_project_key_justifies_a_token_no_event_mentions() {
        // A server that checked only session events would refuse a token the
        // client legitimately justified from an established key, and the
        // refusal is permanent — the decision is destroyed, not deferred.
        let v = SessionVocabulary::new().with_established_keys(["deploy.images", "storage"]);
        assert!(v.contains("deploy"));
        assert!(v.contains("images"));
        assert!(v.contains("deploy.images"));
        assert!(v.contains("storage"));
        assert!(!v.contains("unrelated"));
    }

    #[test]
    fn a_dot_segmented_token_needs_every_segment_justified() {
        let mut v = SessionVocabulary::new();
        v.observe(EventKind::FileChanged, Some(&file("deploy/images.rs")));
        assert!(v.contains("deploy.images"));
        assert!(
            !v.contains("deploy.secrets"),
            "an unjustified segment made the whole token justified"
        );
    }

    #[test]
    fn a_tool_name_is_a_token_and_an_unknown_content_shape_contributes_nothing() {
        let mut v = SessionVocabulary::new();
        v.observe(
            EventKind::ToolSucceeded,
            Some(&EventContent::Tool {
                vendor_tool: "WebFetch".into(),
                tool_class: ToolClass::Research,
            }),
        );
        assert!(v.contains("webfetch"));

        let before = v.len();
        v.observe(EventKind::AgentQuiesced, Some(&EventContent::None));
        v.observe(EventKind::AgentQuiesced, None);
        assert_eq!(v.len(), before, "an empty event widened the vocabulary");
    }

    #[test]
    fn derivation_is_deterministic_and_order_independent_in_its_result() {
        let a = file("crates/cairnd/src/sync.rs");
        let b = EventContent::Command {
            command_line: "cargo test".into(),
            exit_status: Some(0),
        };
        let forwards = derive(
            [
                (EventKind::FileChanged, Some(&a)),
                (EventKind::CommandExecuted, Some(&b)),
            ],
            ["deploy.images"],
        );
        let backwards = derive(
            [
                (EventKind::CommandExecuted, Some(&b)),
                (EventKind::FileChanged, Some(&a)),
            ],
            ["deploy.images"],
        );
        // The *set* does not depend on order. Ordering matters for which events
        // are eligible at all — a signal is justified only by lower
        // `session_seq` — and that filtering is the caller's, not this
        // function's.
        assert_eq!(forwards, backwards);
    }
}
