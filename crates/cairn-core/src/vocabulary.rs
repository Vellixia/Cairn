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
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where a token came from, ordered so that the strongest source is greatest.
///
/// The order is the fixed, total rank `contracts/extraction.md` §13.5 assigns
/// roles by, strongest first: an established project `topic_key`, then an
/// established project `value_key`, then a module token, a file token, a test
/// token, and last a command verb. It is total so the choice of subject cannot
/// depend on iteration order, and it is a rank rather than a score because
/// there is no arithmetic to do — one source is simply more established than
/// another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VocabRank {
    /// The leading word of a shell command, or a vendor tool's name.
    CommandVerb,
    /// A word from a test invocation.
    Test,
    /// The final component of a repository path, without its extension.
    File,
    /// A directory component of a repository path.
    Module,
    /// A `value_key` this project's knowledge already establishes.
    EstablishedValueKey,
    /// A `topic_key` this project's knowledge already establishes.
    EstablishedTopicKey,
}

/// What the vocabulary knows about one token.
///
/// `seq` is the `session_seq` of the **earliest** event that justified it, and
/// is absent for a token an established project key supplied — a key was not
/// established by any event in this session, so it has no ordinal here. That
/// absence is load-bearing twice: it sorts such a token last in the §13.5
/// tiebreak, and it keeps it out of the `justified_by_seq` a refusal would
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabEntry {
    pub rank: VocabRank,
    pub seq: Option<u64>,
}

/// One session's derived vocabulary.
///
/// A map from token to the strongest source that justified it. Membership is
/// what the server asks; the rank is what the client's role assignment needs
/// (`contracts/extraction.md` §13.5). Both read the same structure, so the two
/// sides cannot disagree about which tokens exist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionVocabulary {
    tokens: BTreeMap<String, VocabEntry>,
}

impl SessionVocabulary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed with the topic keys this project's knowledge already establishes.
    ///
    /// Both key sources are required. A server that justified tokens only from
    /// session events would refuse tokens the client legitimately justified
    /// from established project keys — and the refusal is permanent, so the
    /// decision is destroyed rather than deferred
    /// (`contracts/safe-events.md` §7.1 step 7).
    pub fn with_established_keys<'a>(self, keys: impl IntoIterator<Item = &'a str>) -> Self {
        self.with_established(keys, VocabRank::EstablishedTopicKey)
    }

    /// Seed with the value keys this project's knowledge already establishes.
    ///
    /// Separate from [`Self::with_established_keys`] only because the two rank
    /// differently when a role is assigned; for membership they are the same
    /// kind of evidence.
    pub fn with_established_value_keys<'a>(self, keys: impl IntoIterator<Item = &'a str>) -> Self {
        self.with_established(keys, VocabRank::EstablishedValueKey)
    }

    fn with_established<'a>(
        mut self,
        keys: impl IntoIterator<Item = &'a str>,
        rank: VocabRank,
    ) -> Self {
        for key in keys {
            // An established key may be dot-segmented. Both the whole key and
            // each segment count: a decision citing `deploy` is justified by an
            // established `deploy.images`, because the subject is visibly part
            // of this project's vocabulary either way.
            if let Some(normalized) = normalize_topic_key(key) {
                for segment in normalized.split('.') {
                    self.insert(segment, rank, None);
                }
                self.insert(&normalized, rank, None);
            }
        }
        self
    }

    /// Add everything one event contributes, without an ordinal.
    ///
    /// Only events with a **lower `session_seq`** than the signal being checked
    /// should be fed in. A token justified only by a later event is refused:
    /// the machine that built the signal knew the earlier events too, so it
    /// could not legitimately have cited a later one, and accepting it would
    /// make justification depend on delivery order.
    pub fn observe(&mut self, kind: EventKind, content: Option<&EventContent>) {
        self.observe_at(None, kind, content);
    }

    /// Add everything one event contributes, recording which event it was.
    ///
    /// The ordinal is what lets a client name `justified_by_seq` on the signal
    /// it emits, so a server refusal can say what was missing instead of being
    /// a bare mismatch.
    pub fn observe_at(
        &mut self,
        seq: Option<u64>,
        kind: EventKind,
        content: Option<&EventContent>,
    ) {
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
                    self.add_path_segments(path, seq);
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
                self.add_leading_words(command_line, 2, VocabRank::CommandVerb, seq);
            }
            EventContent::TestInvocation { test_command } => {
                // A test command contributes more of itself than a shell
                // command does, because a suite identifier is often the third
                // or fourth word (`cargo test -p cairn-core`).
                self.add_leading_words(test_command, 4, VocabRank::Test, seq);
            }
            EventContent::Tool { vendor_tool, .. }
            | EventContent::ToolFailure { vendor_tool, .. } => {
                self.insert_normalized(vendor_tool, VocabRank::CommandVerb, seq);
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
            .all(|segment| self.tokens.contains_key(segment))
            || self.tokens.contains_key(token)
    }

    /// What justified a token, if anything did.
    ///
    /// A dot-segmented token takes the **weakest** rank and the **earliest**
    /// ordinal among its segments: a compound is only as established as its
    /// least established part, and claiming otherwise would let one strong
    /// segment promote a whole token above what the evidence supports.
    pub fn entry(&self, token: &str) -> Option<VocabEntry> {
        if let Some(found) = self.tokens.get(token) {
            return Some(*found);
        }
        let mut rank: Option<VocabRank> = None;
        let mut seq: Option<u64> = None;
        for segment in token.split('.') {
            let found = self.tokens.get(segment)?;
            rank = Some(rank.map_or(found.rank, |r: VocabRank| r.min(found.rank)));
            seq = match (seq, found.seq) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, b) => b,
            };
        }
        rank.map(|rank| VocabEntry { rank, seq })
    }

    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The tokens, sorted, for a diagnostic that needs to show its working.
    pub fn tokens(&self) -> impl Iterator<Item = &str> {
        self.tokens.keys().map(String::as_str)
    }

    fn add_path_segments(&mut self, path: &str, seq: Option<u64>) {
        let segments: Vec<&str> = path.split('/').collect();
        let last = segments.len().saturating_sub(1);
        for (i, segment) in segments.iter().enumerate() {
            // A file's extension is not part of its identity here: `sync.rs`
            // and `sync.py` name the same module in two languages, and `rs` is
            // not a subject anyone decides about.
            let stem = segment.rsplit_once('.').map_or(*segment, |(stem, _)| stem);
            // Directory components name a module; the final component names a
            // file. A module outranks a file because it is the coarser subject,
            // and a decision is more often about an area than about one file.
            let rank = if i == last {
                VocabRank::File
            } else {
                VocabRank::Module
            };
            self.insert_normalized(stem, rank, seq);
        }
    }

    fn add_leading_words(
        &mut self,
        text: &str,
        how_many: usize,
        rank: VocabRank,
        seq: Option<u64>,
    ) {
        for word in text.split_whitespace().take(how_many) {
            // Flags are not verbs. `-p` and `--release` say how, not what.
            if word.starts_with('-') {
                continue;
            }
            // A path-shaped invocation contributes its final component:
            // `./scripts/deploy.sh` is `deploy`.
            let last = word.rsplit('/').next().unwrap_or(word);
            let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);
            self.insert_normalized(stem, rank, seq);
        }
    }

    fn insert_normalized(&mut self, raw: &str, rank: VocabRank, seq: Option<u64>) {
        if let Some(normalized) = normalize_topic_key(raw) {
            for segment in normalized.split('.') {
                self.insert(segment, rank, seq);
            }
        }
    }

    fn insert(&mut self, token: &str, rank: VocabRank, seq: Option<u64>) {
        if token.is_empty() {
            return;
        }
        // The strongest source wins, and at equal strength the earliest event
        // does. Both halves are deterministic on purpose: the rank decides the
        // role, the ordinal breaks the tie, and neither depends on the order
        // events happened to be fed in.
        match self.tokens.get_mut(token) {
            Some(existing) if rank > existing.rank => {
                existing.rank = rank;
                existing.seq = seq;
            }
            Some(existing) if rank == existing.rank => {
                existing.seq = match (existing.seq, seq) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (Some(a), None) => Some(a),
                    (None, b) => b,
                };
            }
            Some(_) => {}
            None => {
                self.tokens
                    .insert(token.to_string(), VocabEntry { rank, seq });
            }
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
