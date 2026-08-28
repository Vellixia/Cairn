//! The one implementation of the global-content rejection classes
//! (D433, D446, D447, FR-544–FR-549, FR-577–FR-580).
//!
//! Every path capable of creating personal or team knowledge calls
//! [`validate_global_content`], and there are **five** of them (FR-545): direct
//! personal creation, personal promotion, team proposal, team promotion, and
//! server-side synchronization ingest. Four are client-side; the fifth is not,
//! and that is deliberate — a privacy boundary enforced only where the client
//! chooses to enforce it is a convention, not a boundary.
//!
//! Two properties of this module are load-bearing:
//!
//! **It is pure and total.** No database handle, no clock, no network. Every
//! input shape has a defined answer, including the degenerate ones — empty
//! `applicability`, `None` keys, an empty `project_identities`. That is what
//! lets the whole class list be exercised against a seeded adversarial corpus
//! with no store behind it (SC-421).
//!
//! **It is the only implementation of these classes** (FR-579). Not the
//! promotion gate, which delegates; not the server's ingest handler, which calls
//! this; not a client-side pre-check. A second implementation is a second place
//! for the two to drift, and `tests/tests/global_content_validation.rs` audits
//! for one in a way that fails when a duplicate is introduced (SC-453).
//!
//! Nothing here is Layer A. Everything this module guards — `content`, a topic
//! key, a value key, an applicability value — is free text that can hold
//! anything; it is kept clean by rule, not by absence (FR-550). "Structurally
//! impossible" belongs only to the columns that do not exist.

use crate::domain::ApplicabilityFact;

/// One token identifying a project, to screen **against** (D446, FR-546).
///
/// A project's name, a path component of its shared identity
/// (`git_common_dir`), or a remote host, organisation or repository token. This
/// is the set being matched against, never the value being matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdentity(pub String);

/// Why global content was refused.
///
/// Carries a **class only**, and that is structural rather than a promise the
/// caller must keep: there is no field on this type into which the offending
/// text could be placed, so a caller cannot echo, log or return it by accident
/// (FR-547, SC-439).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalContentRejection {
    /// One of [`CONTENT_CLASSES`], or [`INVALID_APPLICABILITY`].
    pub class: &'static str,
}

impl GlobalContentRejection {
    fn of(class: &'static str) -> Self {
        Self { class }
    }
}

impl std::fmt::Display for GlobalContentRejection {
    /// Prints the class and nothing else. The `Debug` derive is safe for the
    /// same reason: the struct has one field and it is a fixed string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.class)
    }
}

/// The nine content classes, in the order checked. First match wins, so the
/// reported reason is stable for a given input.
///
/// Named here once so that a test can assert the corpus covers every one of
/// them: a class added without a corresponding corpus entry leaves SC-421
/// unmet rather than silently unverified.
pub const CONTENT_CLASSES: &[&str] = &[
    "absolute_path",
    "home_dir_ref",
    "drive_letter_path",
    "file_uri",
    "credentialed_url",
    "env_assignment",
    "encoded_secret_shape",
    "project_identifying",
    "command_shaped",
];

/// An applicability pair whose *format* is wrong — a kind outside the closed
/// vocabulary, or a value that is not `[a-z0-9_]{1,64}` after normalization.
///
/// Distinct from the nine classes: those describe what a value *says*, this
/// describes what it *is*. Both travel through the same `Result` and the same
/// type, so no caller needs to branch on which kind of refusal it got.
pub const INVALID_APPLICABILITY: &str = "invalid_applicability";

/// Screen everything a personal or team record can carry in free text
/// (FR-544, FR-546, FR-578).
///
/// `project_identities` is the set of tokens the `project_identifying` class
/// matches against, and what it holds depends on the entry point: the source
/// project's tokens at a promotion, the current project's at a direct creation
/// or a team proposal, and the union over the pushing user's memberships at
/// server-side ingest (D447).
///
/// # The one exception to fail-closed
///
/// An **empty** `project_identities` slice **passes** the `project_identifying`
/// check (FR-580). A check with nothing to match is *vacuous*, not
/// *unevaluable*, and those are different things. Implementing it fail-closed
/// would refuse every global creation made outside a linked project — the normal
/// case for cross-project personal knowledge, and the situation the feature
/// exists for.
///
/// A check that genuinely **cannot** be evaluated still fails closed (FR-549).
/// In a pure function over strings there is exactly one such case, and it is
/// distinguishable from the vacuous one: a `project_identities` slice that is
/// non-empty but contains a token which is blank or unusable after trimming.
/// That means the caller believed it had a project identity and passed
/// something that cannot be screened against — an answer of "no match" would be
/// a guess. Empty slice: the caller says there is no identity, and is believed.
/// Blank token inside a non-empty slice: the caller says there is one, and it is
/// not usable, so the check refuses.
pub fn validate_global_content(
    content: &str,
    topic_key: Option<&str>,
    value_key: Option<&str>,
    applicability: &[ApplicabilityFact],
    project_identities: &[ProjectIdentity],
) -> Result<(), GlobalContentRejection> {
    // FR-549's genuinely-unevaluable case, checked before anything it would
    // affect. A caller that supplied an identity slot it could not fill gets a
    // refusal rather than a check that quietly answers "no match".
    if project_identities
        .iter()
        .any(|identity| identity.0.trim().is_empty())
    {
        return Err(GlobalContentRejection::of("evaluation_incomplete"));
    }

    for field in [Some(content), topic_key, value_key].into_iter().flatten() {
        screen(field, project_identities)?;
    }

    for fact in applicability {
        // Stage one: is this pair even representable? A kind outside the
        // vocabulary cannot reach here — it is not constructible — but a value
        // that fails normalization can.
        let normalized = crate::applicability::normalize_applicability_value(&fact.value)
            .map_err(|_| GlobalContentRejection::of(INVALID_APPLICABILITY))?;
        // Stage two, and this is FR-578: the value is then screened as if it
        // were content. `tool = "acme-internal-deploy"` names a project as
        // surely as any sentence does, and it used to travel in a field nothing
        // checked because its *kind* came from a closed vocabulary.
        screen(&normalized, project_identities)?;
        screen(&fact.value, project_identities)?;
    }

    Ok(())
}

/// One free-text field against all nine classes, in order.
fn screen(
    text: &str,
    project_identities: &[ProjectIdentity],
) -> Result<(), GlobalContentRejection> {
    if let Some(class) = matched_class(text, project_identities) {
        return Err(GlobalContentRejection::of(class));
    }
    Ok(())
}

/// The first class `text` matches, if any.
///
/// Split out from [`screen`] so a test can assert *which* class matched without
/// the rejection type ever carrying the text.
pub fn matched_class(text: &str, project_identities: &[ProjectIdentity]) -> Option<&'static str> {
    if has_absolute_path(text) {
        return Some("absolute_path");
    }
    if has_home_dir_ref(text) {
        return Some("home_dir_ref");
    }
    if has_drive_letter_path(text) {
        return Some("drive_letter_path");
    }
    if has_file_uri(text) {
        return Some("file_uri");
    }
    if has_credentialed_url(text) {
        return Some("credentialed_url");
    }
    if has_env_assignment(text) {
        return Some("env_assignment");
    }
    if has_encoded_secret_shape(text) {
        return Some("encoded_secret_shape");
    }
    // Vacuous when the slice is empty (FR-580); the unevaluable case was
    // already refused by the caller.
    if names_a_project(text, project_identities) {
        return Some("project_identifying");
    }
    if has_command_shape(text) {
        return Some("command_shaped");
    }
    None
}

/// Tokens, for the checks that need word boundaries rather than substrings.
fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '"' | '\'' | ',' | ';'))
        .filter(|t| !t.is_empty())
}

fn has_absolute_path(text: &str) -> bool {
    // **A rooted token with anything after the slash.** FR-546 defines the class
    // as an absolute filesystem path, and `/tmp`, `/etc`, `/workspace` and
    // `/srv` are absolute filesystem paths. The old shape demanded a *second*
    // slash, so every single-component root passed all five entry points and
    // could be persisted and synchronized — the most commonly written absolute
    // paths there are, and the ones a reader is most likely to type from memory.
    //
    // The two exclusions the second-slash rule was standing in for are kept
    // explicitly, because they are the only prose this shape catches by accident:
    // `/` alone is not a path, and a token whose slash is internal (`and/or`,
    // `read/write`) never starts with one, so it was never in scope here.
    tokens(text).any(|t| {
        let t = t.trim_end_matches(['.', ',', ':', ';', '!', '?', ')', ']']);
        t.len() > 1 && t.starts_with('/')
    })
}

fn has_home_dir_ref(text: &str) -> bool {
    tokens(text).any(|t| t.starts_with("~/") || t == "~" || t.starts_with("$HOME"))
}

fn has_drive_letter_path(text: &str) -> bool {
    tokens(text).any(|t| {
        let bytes = t.as_bytes();
        bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/')
    })
}

fn has_file_uri(text: &str) -> bool {
    text.to_ascii_lowercase().contains("file://")
}

fn has_credentialed_url(text: &str) -> bool {
    // `scheme://user:pass@host`. The `:` between user and pass is what
    // separates a credentialed URL from an ordinary `git@host` remote, which is
    // handled by `names_a_project` when it names a project and is otherwise not
    // a secret.
    tokens(text).any(|t| {
        let Some((scheme, rest)) = t.split_once("://") else {
            return false;
        };
        if scheme.is_empty() {
            return false;
        }
        let Some((userinfo, _)) = rest.split_once('@') else {
            return false;
        };
        userinfo.contains(':') && !userinfo.is_empty()
    })
}

fn has_env_assignment(text: &str) -> bool {
    // An all-caps identifier immediately followed by `=` and a value.
    tokens(text).any(|t| {
        let Some((name, value)) = t.split_once('=') else {
            return false;
        };
        !name.is_empty()
            && !value.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            && name.chars().any(|c| c.is_ascii_uppercase())
    })
}

fn has_encoded_secret_shape(text: &str) -> bool {
    // A long unbroken run of base64/hex alphabet with the mixed-case or
    // digit-density that prose does not have. Deliberately conservative: the
    // aim is a run no sentence produces.
    const MIN: usize = 32;
    tokens(text).any(|t| {
        let t = t.trim_end_matches(['.', ',', ':', ';', '!', '?']);
        if t.len() < MIN {
            return false;
        }
        if !t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-'))
        {
            return false;
        }
        let digits = t.chars().filter(|c| c.is_ascii_digit()).count();
        let upper = t.chars().filter(|c| c.is_ascii_uppercase()).count();
        let lower = t.chars().filter(|c| c.is_ascii_lowercase()).count();
        // Hex-ish, or mixed case with digits. Plain lower-case words of 32+
        // characters are not secrets and should not be refused.
        let hexish = t.chars().all(|c| c.is_ascii_hexdigit());
        hexish || (digits > 0 && upper > 0 && lower > 0)
    })
}

fn names_a_project(text: &str, project_identities: &[ProjectIdentity]) -> bool {
    if project_identities.is_empty() {
        // Vacuous, and that is the documented exception (FR-580).
        return false;
    }
    let haystack = fold_separators(text);
    project_identities.iter().any(|identity| {
        let needle = fold_separators(identity.0.trim());
        // A one- or two-character identity would match half of all prose. One
        // that short is not usable as a screen, and refusing on it would be
        // worse than not screening at all.
        needle.chars().count() >= 3 && haystack.contains(&needle)
    })
}

/// Lower-case, and drop everything that is not a letter or a digit.
///
/// Without this the check is trivially evaded, and by accident as often as on
/// purpose: a project named `acme-widgets` is not matched by `acme_widgets`,
/// `acme.widgets` or `acmewidgets` under a plain substring test. The
/// applicability path makes that worse than a theoretical gap — an applicability
/// value must be `[a-z0-9_]` after normalization, so a hyphenated project name
/// *cannot* appear there in its original spelling, and the screen would have
/// been unable to fire on any project whose name contains a hyphen or a dot.
/// Folding both sides is what makes "by project name" (FR-546) mean the name
/// rather than one spelling of it.
fn fold_separators(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn has_command_shape(text: &str) -> bool {
    // Shell syntax that appears in no sentence. These are conclusive on their
    // own: nothing but a command line contains `&&` or a substitution.
    const SHELL_ONLY: &[&str] = &["&&", "||", "$(", "`", "|&", ";;", ">>", "2>&1", " | "];
    if SHELL_ONLY.iter().any(|m| text.contains(m)) {
        return true;
    }

    // Beyond that, the distinction is **prose mentioning a tool** versus
    // **content that is an invocation**, and it is not the presence of a flag.
    //
    // An earlier version of this function asked "is there a flag or a path after
    // the command name" and treated everything else as prose. That was a
    // heuristic standing in for the real question, and it let six plain commands
    // through — `cargo test`, `rm target`, `sudo reboot`, `npm install`,
    // `git status`, `cat /etc/passwd`. Every one of them is a command by any
    // reading, and none of them carries a flag.
    //
    // What actually separates the two is **grammatical position**. A tool named
    // in prose sits inside a sentence, with a preposition or a copula around it:
    // "Use cargo nextest for the test suite", "docker is the deployment target".
    // An invocation puts the program in imperative head position with its
    // operands after it: "cargo test", "rm -rf ./target".
    for clause in text.split(['\n', ';']).flat_map(|c| c.split(". ")) {
        if clause_is_an_invocation(clause) {
            return true;
        }
    }
    false
}

/// Programs whose name in head position means "run this".
const COMMANDS: &[&str] = &[
    "sudo",
    "rm",
    "mv",
    "cp",
    "cat",
    "chmod",
    "chown",
    "curl",
    "wget",
    "ssh",
    "scp",
    "docker",
    "kubectl",
    "psql",
    "git",
    "npm",
    "pnpm",
    "yarn",
    "cargo",
    "make",
    "bash",
    "sh",
    "zsh",
    "python",
    "python3",
    "node",
    "pip",
    "apt",
    "brew",
    "systemctl",
    "export",
    "eval",
    "kill",
    "pkill",
    "tar",
    "unzip",
    "dd",
    "mkfs",
    "reboot",
    "shutdown",
];

/// Verbs that mean "invoke a program", as opposed to "adopt this tool".
///
/// `run` and `execute` are here; `use` and `prefer` are deliberately not.
/// "Use cargo nextest for the test suite" is guidance about which tool to adopt
/// — `contracts/promotion-privacy.md` uses that exact sentence as its passing
/// example — while "Run cargo nextest" is an instruction to execute something.
/// The difference is real, not a concession to make a test pass.
const INVOKING_VERBS: &[&str] = &[
    "run", "runs", "running", "execute", "executes", "exec", "invoke",
];

/// English function words. An operand is never one of these.
///
/// This is what keeps "docker is the deployment target here" prose: `docker` is
/// in head position, but what follows it is a copula, and no invocation reads
/// `docker is`.
const FUNCTION_WORDS: &[&str] = &[
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "and",
    "or",
    "but",
    "for",
    "with",
    "without",
    "over",
    "under",
    "in",
    "on",
    "at",
    "to",
    "from",
    "as",
    "than",
    "then",
    "the",
    "a",
    "an",
    "of",
    "if",
    "when",
    "while",
    "because",
    "so",
    "can",
    "cannot",
    "should",
    "shouldnt",
    "will",
    "wont",
    "would",
    "may",
    "might",
    "must",
    "does",
    "doesnt",
    "do",
    "dont",
    "has",
    "have",
    "had",
    "its",
    "it",
    "this",
    "that",
    "these",
    "those",
    "here",
    "there",
    "always",
    "never",
    "only",
    "also",
    "just",
    "still",
    "already",
    "instead",
    "rather",
    "prefer",
    "prefers",
    "preferred",
    "use",
    "uses",
    "used",
    "using",
    "supports",
    "provides",
    "requires",
    "needs",
    "handles",
    "works",
    "gives",
    "keeps",
    "stays",
];

/// Is this clause an invocation rather than a sentence mentioning a tool?
fn clause_is_an_invocation(clause: &str) -> bool {
    let words: Vec<&str> = tokens(clause).collect();
    for (i, word) in words.iter().enumerate() {
        let head = word
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .to_ascii_lowercase();
        if !COMMANDS.contains(&head.as_str()) {
            continue;
        }

        // A flag, a path, or a redirect anywhere in the tail settles it — those
        // do not occur in prose about a tool.
        let operand_shaped = words.iter().skip(i + 1).take(4).any(|t| {
            (t.starts_with('-') && t.len() > 1)
                || t.contains('/')
                || t.starts_with('>')
                || t.starts_with('<')
        });
        if operand_shaped {
            return true;
        }

        // An explicit invoking verb settles it on its own: "Run make" is an
        // instruction, and nothing needs to follow the program name for it to be
        // one. This is the case a rule requiring an operand gets wrong.
        let preceded_by_invocation = !words[..i].is_empty()
            && words[..i].iter().all(|w| {
                let w = w
                    .trim_matches(|c: char| !c.is_ascii_alphanumeric())
                    .to_ascii_lowercase();
                INVOKING_VERBS.contains(&w.as_str()) || w == "$" || w == "#" || w == "then"
            });
        if preceded_by_invocation {
            return true;
        }
        if i != 0 {
            // Anywhere else in a sentence, with no operand and no invoking verb,
            // the program name is a noun.
            continue;
        }

        // Clause-initial and no invoking verb: ambiguous between "cargo test"
        // and "docker is the deployment target". A subcommand or operand after
        // it makes it an invocation; a function word, or nothing at all, makes
        // it a sentence about the tool.
        let Some(next) = words.get(i + 1) else {
            continue;
        };
        let next = next
            .trim_matches(|c: char| !c.is_ascii_alphanumeric())
            .to_ascii_lowercase();
        if next.is_empty() || FUNCTION_WORDS.contains(&next.as_str()) {
            continue;
        }
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ApplicabilityFact, ApplicabilityKind};

    fn identities(tokens: &[&str]) -> Vec<ProjectIdentity> {
        tokens
            .iter()
            .map(|t| ProjectIdentity(t.to_string()))
            .collect()
    }

    fn check(content: &str, ids: &[ProjectIdentity]) -> Result<(), GlobalContentRejection> {
        validate_global_content(content, None, None, &[], ids)
    }

    /// One case per class, each asserted to fail **on its own class** rather
    /// than merely to fail. A test that only asserted `is_err()` would pass on
    /// an implementation that refused everything, which is the failure mode
    /// worth guarding against here.
    ///
    /// The corpus is driven from [`CONTENT_CLASSES`] so that adding a class
    /// without adding a case makes this test fail — SC-421 requires the corpus
    /// to cover every class the validator declares, and a hand-written list
    /// would silently fall behind.
    #[test]
    fn every_declared_class_has_a_case_that_trips_exactly_it() {
        let project = identities(&["acme-widgets"]);
        let corpus: &[(&str, &str)] = &[
            ("absolute_path", "Scratch files live at /Users/alice/tmp"),
            ("home_dir_ref", "The cache sits in ~/Library/Caches/thing"),
            ("drive_letter_path", r"Logs are at C:\Users\alice\logs"),
            ("file_uri", "See file:///etc/hosts for the mapping"),
            (
                "credentialed_url",
                "Mirror is at https://user:s3cret@mirror.example/repo",
            ),
            (
                "env_assignment",
                "Set DATABASE_URL=postgres://x to override",
            ),
            (
                "encoded_secret_shape",
                "Token 4f2a1c9e8b7d6a5f4e3d2c1b0a998877 rotates monthly",
            ),
            ("project_identifying", "The acme-widgets build is slow"),
            ("command_shaped", "Run cargo nextest run --workspace first"),
        ];

        let covered: Vec<&str> = corpus.iter().map(|(class, _)| *class).collect();
        for class in CONTENT_CLASSES {
            assert!(
                covered.contains(class),
                "class {class:?} is declared but has no corpus case (SC-421)"
            );
        }

        for (class, text) in corpus {
            let err = check(text, &project).expect_err(&format!("{class}: {text:?} was accepted"));
            assert_eq!(
                err.class, *class,
                "{text:?} matched {:?}, expected {class}",
                err.class
            );
        }
    }

    /// Prose that resembles a refused shape but is not one. Without this, every
    /// detector above could be `true` and the suite would still pass.
    #[test]
    fn ordinary_guidance_is_not_refused() {
        let no_project: [ProjectIdentity; 0] = [];
        for benign in [
            "Prefer thiserror over hand-rolled Display impls",
            "Retry flaky integration tests up to three times",
            "Use cargo for the test suite",
            "Clear the build cache when a stale artifact is suspected",
            "Wrap at 95 columns",
            "and/or is not a path",
            "The ratio is 3/4",
            // The contract's own passing example for the project-identity
            // check, which an earlier draft of `has_command_shape` refused
            // because `cargo` was followed by an ordinary word.
            "Use cargo nextest for the test suite",
            "Prefer git over a bespoke tool",
            "docker is the deployment target here",
        ] {
            assert!(
                check(benign, &no_project).is_ok(),
                "{benign:?} was refused: {:?}",
                check(benign, &no_project).unwrap_err()
            );
        }
    }

    /// FR-580, and its own test on purpose.
    ///
    /// An **empty** identity set passes the `project_identifying` check. A check
    /// with nothing to match is *vacuous*. Implementing it fail-closed would
    /// refuse every global creation made outside a linked project, which is the
    /// normal case for cross-project personal knowledge.
    #[test]
    fn an_empty_project_identity_set_passes_rather_than_refusing() {
        let none: [ProjectIdentity; 0] = [];
        assert!(check("The acme-widgets build is slow", &none).is_ok());
        // The same content *is* refused once an identity exists to match, which
        // is what proves the pass above is the empty-set rule and not a broken
        // detector.
        assert_eq!(
            check(
                "The acme-widgets build is slow",
                &identities(&["acme-widgets"])
            )
            .unwrap_err()
            .class,
            "project_identifying"
        );
    }

    /// FR-549, and a **separate** test from the one above, because they are
    /// separate behaviors that an implementation can easily conflate.
    ///
    /// An identity slice that is non-empty but holds a blank token means the
    /// caller believed it had a project identity and supplied something
    /// unusable. Answering "no match" would be a guess, so the check refuses.
    #[test]
    fn an_unevaluable_project_identity_fails_closed() {
        for unusable in [vec![""], vec!["   "], vec!["acme-widgets", ""]] {
            let err = check("perfectly ordinary guidance", &identities(&unusable))
                .expect_err(&format!("{unusable:?} was treated as evaluable"));
            assert_eq!(err.class, "evaluation_incomplete", "{unusable:?}");
        }
    }

    /// FR-578 / SC-448 — an applicability **value** is screened by the same nine
    /// classes as content, not merely constrained to a format.
    ///
    /// Distinct from the vocabulary check: the *kind* comes from a closed
    /// enum, and that is what used to be mistaken for the value being safe.
    #[test]
    fn an_applicability_value_is_screened_as_content() {
        let project = identities(&["acme_internal"]);
        let naming_the_project = [ApplicabilityFact {
            kind: ApplicabilityKind::Tool,
            value: "acme_internal".to_string(),
        }];
        let err = validate_global_content(
            "harmless guidance",
            None,
            None,
            &naming_the_project,
            &project,
        )
        .expect_err("a project-identifying applicability value was accepted");
        assert_eq!(err.class, "project_identifying");

        // And the same string refused identically when it appears in content
        // instead — one implementation, one answer (FR-579).
        assert_eq!(
            check("acme_internal is the deploy tool", &project)
                .unwrap_err()
                .class,
            "project_identifying"
        );
    }

    /// A malformed applicability value is refused under its own class, because
    /// what is wrong with it is its *format*, not what it says.
    #[test]
    fn a_malformed_applicability_value_is_refused_as_invalid_applicability() {
        let none: [ProjectIdentity; 0] = [];
        let bad = [ApplicabilityFact {
            kind: ApplicabilityKind::Language,
            value: "has space".to_string(),
        }];
        assert_eq!(
            validate_global_content("fine", None, None, &bad, &none)
                .unwrap_err()
                .class,
            INVALID_APPLICABILITY
        );
    }

    /// The topic key and value key are screened too — they are free text on the
    /// same row and were not covered by an earlier draft that screened only
    /// `content`.
    #[test]
    fn the_subject_keys_are_screened_as_well_as_the_content() {
        let none: [ProjectIdentity; 0] = [];
        assert_eq!(
            validate_global_content("fine", Some("/etc/passwd"), None, &[], &none)
                .unwrap_err()
                .class,
            "absolute_path"
        );
        assert_eq!(
            validate_global_content("fine", None, Some("~/secrets"), &[], &none)
                .unwrap_err()
                .class,
            "home_dir_ref"
        );
    }

    /// FR-547 / SC-439 — the rejection carries the class and nothing else.
    ///
    /// Asserted across **all nine** classes and against both `Debug` and
    /// `Display`, because a leak through either is a leak. The struct has one
    /// field, so this holds by construction; the test exists so that adding a
    /// second field fails rather than quietly widening what a rejection can
    /// carry.
    #[test]
    fn a_rejection_never_carries_the_offending_text() {
        let project = identities(&["acme-widgets"]);
        let corpus: &[&str] = &[
            "Scratch files live at /Users/alice/tmp",
            "The cache sits in ~/Library/Caches/thing",
            r"Logs are at C:\Users\alice\logs",
            "See file:///etc/hosts for the mapping",
            "Mirror is at https://user:s3cret@mirror.example/repo",
            "Set DATABASE_URL=postgres://x to override",
            "Token 4f2a1c9e8b7d6a5f4e3d2c1b0a998877 rotates monthly",
            "The acme-widgets build is slow",
            "Run cargo nextest run --workspace first",
        ];
        for text in corpus {
            let err = check(text, &project).expect_err(text);
            let debug = format!("{err:?}");
            let display = format!("{err}");
            for rendering in [&debug, &display] {
                assert!(
                    CONTENT_CLASSES.contains(&err.class),
                    "unknown class {:?}",
                    err.class
                );
                // No token of the input longer than two characters may appear.
                for token in text.split_whitespace().filter(|t| t.len() > 2) {
                    assert!(
                        !rendering.contains(token),
                        "rejection rendering {rendering:?} leaked {token:?} from {text:?}"
                    );
                }
            }
        }
    }

    /// Purity, asserted the only way a test can: the same inputs give the same
    /// answer, and the function needs nothing but its arguments.
    #[test]
    fn the_validator_is_total_and_deterministic() {
        let none: [ProjectIdentity; 0] = [];
        for _ in 0..3 {
            assert!(validate_global_content("", None, None, &[], &none).is_ok());
        }
        // Every degenerate shape has an answer rather than a panic.
        assert!(validate_global_content("", Some(""), Some(""), &[], &none).is_ok());
    }
}

#[cfg(test)]
mod separator_folding_tests {
    use super::*;

    /// A project name is matched however it is spelled (FR-546).
    ///
    /// The applicability path forces the `[a-z0-9_]` spelling, so a screen that
    /// compared raw substrings could never fire for a project whose name
    /// contains a hyphen or a dot — which is most of them. Each row here is a
    /// spelling an author could reach for without meaning to evade anything.
    #[test]
    fn a_hyphenated_project_name_is_matched_however_it_is_spelled() {
        let identity = [ProjectIdentity("acme-widgets".to_string())];
        for spelling in [
            "acme-widgets is slow",
            "acme_widgets is slow",
            "acme.widgets is slow",
            "acmewidgets is slow",
            "ACME-Widgets is slow",
            "Acme Widgets is slow",
        ] {
            assert_eq!(
                validate_global_content(spelling, None, None, &[], &identity)
                    .unwrap_err()
                    .class,
                "project_identifying",
                "{spelling:?} evaded the screen"
            );
        }
    }

    /// Folding must not make the screen match everything. An unrelated project
    /// name is still unrelated after folding.
    #[test]
    fn folding_does_not_turn_the_screen_into_a_wildcard() {
        let identity = [ProjectIdentity("acme-widgets".to_string())];
        for benign in [
            "Prefer thiserror over hand-rolled Display impls",
            "widgets are a useful abstraction",
            "acme is a common placeholder",
        ] {
            assert!(
                validate_global_content(benign, None, None, &[], &identity).is_ok(),
                "{benign:?} was refused"
            );
        }
    }
}

#[cfg(test)]
mod adversarial_command_shape_tests {
    use super::*;

    fn refused(text: &str) -> bool {
        let none: [ProjectIdentity; 0] = [];
        matches!(
            validate_global_content(text, None, None, &[], &none),
            Err(GlobalContentRejection {
                class: "command_shaped"
            })
        )
    }

    /// The table that replaced the heuristic.
    ///
    /// The rule was "a command name followed by a flag or a path". It let six
    /// plain commands through — every one of them a command by any reading, and
    /// none of them carrying a flag. The rows marked `true` below are the ones
    /// that used to pass; they are the reason this table exists rather than a
    /// single assertion.
    #[test]
    fn an_invocation_is_refused_and_a_mention_is_not() {
        let cases: &[(&str, bool, &str)] = &[
            // ---- prose: a tool named inside a sentence -------------------
            (
                "Use cargo nextest for the test suite",
                false,
                "the contract's own passing example",
            ),
            ("Use cargo for the test suite", false, "same, shorter"),
            ("Prefer git over a bespoke tool", false, "preposition after"),
            (
                "docker is the deployment target here",
                false,
                "copula after",
            ),
            ("The git history is long", false, "not in head position"),
            ("make is available on every runner", false, "copula after"),
            ("Prefer make over cargo-make", false, "preposition after"),
            (
                "Retry flaky integration tests three times",
                false,
                "no command at all",
            ),
            (
                "Clear the build cache when an artifact is stale",
                false,
                "no command",
            ),
            // ---- invocations the old rule let through --------------------
            ("cargo test", true, "head position, subcommand after"),
            ("rm target", true, "destructive, operand, no flag"),
            ("sudo reboot", true, "head position"),
            ("npm install", true, "head position"),
            ("git status", true, "head position"),
            (
                "docker compose down",
                true,
                "head position, not a copula after",
            ),
            // ---- invocations the old rule already caught -----------------
            ("cargo test --workspace", true, "flag"),
            (
                "Run cargo nextest run --workspace first",
                true,
                "flag, mid-sentence",
            ),
            ("Clear it with rm -rf ./target", true, "flag and path"),
            ("python3 -m venv .venv", true, "flag"),
            // ---- shell syntax, conclusive on its own ---------------------
            ("build && test", true, "shell operator"),
            ("echo $(whoami)", true, "command substitution"),
            ("make 2>&1", true, "redirect"),
            // ---- an invoking verb makes position explicit ---------------
            ("Run make", true, "an invoking verb, then the program"),
            ("Execute psql", true, "same"),
        ];

        for (text, should_refuse, why) in cases {
            assert_eq!(
                refused(text),
                *should_refuse,
                "{text:?} ({why}): expected refuse={should_refuse}, got {}",
                refused(text)
            );
        }
    }

    /// "Requires a flag or path" must not come back as the rule.
    ///
    /// Asserted directly, because that heuristic is the natural thing to reach
    /// for and it reads as reasonable until you list what it admits.
    #[test]
    fn a_command_without_a_flag_or_a_path_is_still_a_command() {
        for bare in ["cargo test", "rm target", "git status", "npm install"] {
            assert!(
                refused(bare),
                "{bare:?} passed; the flag-or-path heuristic has returned"
            );
        }
    }
}

#[cfg(test)]
mod adversarial_project_identity_tests {
    use super::*;

    fn class_of(text: &str, identity: &str) -> Option<&'static str> {
        let ids = [ProjectIdentity(identity.to_string())];
        validate_global_content(text, None, None, &[], &ids)
            .err()
            .map(|e| e.class)
    }

    /// Bypasses the separator folding closes, and false positives it must not
    /// introduce.
    ///
    /// Folding exists because the applicability path forces the `[a-z0-9_]`
    /// spelling: a project named `acme-widgets` literally cannot appear there in
    /// its own spelling, so a raw substring screen could never fire for any
    /// hyphenated or dotted project name — which is most of them.
    #[test]
    fn a_project_name_is_matched_however_it_is_spelled_and_only_then() {
        let identity = "acme-widgets";
        let cases: &[(&str, bool, &str)] = &[
            // ---- must be refused: the same name, differently written -----
            ("acme-widgets is slow", true, "exact"),
            ("acme_widgets is slow", true, "hyphen to underscore"),
            ("acme.widgets is slow", true, "hyphen to dot"),
            ("acmewidgets is slow", true, "separator removed"),
            ("ACME-WIDGETS is slow", true, "upper case"),
            ("Acme Widgets is slow", true, "spaced and title-cased"),
            (
                "the acme-widgets build is slow",
                true,
                "embedded in a sentence",
            ),
            ("see acme--widgets", true, "doubled separator"),
            // ---- must NOT be refused: unrelated text ---------------------
            (
                "Prefer thiserror over hand-rolled Display impls",
                false,
                "unrelated",
            ),
            ("widgets are a useful abstraction", false, "one half only"),
            ("acme is a common placeholder", false, "the other half only"),
            ("wid gets are not widgets", false, "no match"),
            ("acmewidget is singular", false, "prefix, not the name"),
        ];
        for (text, should_refuse, why) in cases {
            let got = class_of(text, identity);
            let refused = got == Some("project_identifying");
            assert_eq!(
                refused, *should_refuse,
                "{text:?} ({why}): expected refuse={should_refuse}, got {got:?}"
            );
        }
    }

    /// A short identity is not used as a screen at all.
    ///
    /// A project named `ci` would otherwise refuse most English prose. Under-
    /// screening here is the lesser error: a two-character token carries no
    /// information about whether the content is about that project.
    #[test]
    fn an_identity_too_short_to_be_meaningful_is_not_screened_on() {
        assert_eq!(class_of("the ci pipeline is slow", "ci"), None);
        assert_eq!(class_of("an api is a contract", "ap"), None);
        // Three characters is the threshold, and it does screen.
        assert_eq!(
            class_of("the abc pipeline is slow", "abc"),
            Some("project_identifying")
        );
    }

    /// Folding over-refuses across a sentence boundary, and this records it
    /// rather than leaving it to be rediscovered.
    ///
    /// Deleting separators means "…the acme. Widgets are fine…" folds to a
    /// string containing `acmewidgets`. That is a false positive, and it is the
    /// accepted direction of error: over-refusing a promotion costs an author
    /// one rewrite, while under-refusing puts a project name on the wire. The
    /// alternative — folding to a separator instead of deleting — would let
    /// `acmewidgets` through, which is a bypass anyone could reach by accident.
    #[test]
    fn folding_over_refuses_across_a_sentence_boundary_by_design() {
        assert_eq!(
            class_of("Use acme. Widgets are fine.", "acme-widgets"),
            Some("project_identifying"),
            "if this stops refusing, check that the separator-deleting fold is still in place"
        );
    }

    /// FR-580's exception, restated here beside the bypass cases so the two are
    /// read together: an empty identity set passes, and that is the *only*
    /// sanctioned fail-open in this module.
    #[test]
    fn an_empty_identity_set_is_the_only_fail_open() {
        let none: [ProjectIdentity; 0] = [];
        assert!(
            validate_global_content("acme-widgets is slow", None, None, &[], &none).is_ok(),
            "the documented exception (FR-580) no longer holds"
        );
        // And a blank token inside a non-empty set is not the same thing.
        let blank = [ProjectIdentity("  ".to_string())];
        assert_eq!(
            validate_global_content("anything", None, None, &[], &blank)
                .unwrap_err()
                .class,
            "evaluation_incomplete"
        );
    }
}

#[cfg(test)]
mod rooted_path_tests {
    use super::*;

    /// A single-component root is an absolute path (FR-546).
    ///
    /// The detector demanded a *second* slash, so `/tmp`, `/etc`, `/srv` and
    /// `/workspace` passed all five entry points and could be persisted and
    /// synchronized — the most commonly written absolute paths there are.
    ///
    /// Falsified by restoring the `t[1..].contains('/')` condition.
    #[test]
    fn a_single_component_root_is_refused() {
        for content in [
            "the cache lives in /tmp",
            "config is under /etc",
            "we mount /workspace",
            "look in /srv.",
            "see /var, then /opt",
            "the build writes to /out)",
        ] {
            assert_eq!(
                validate_global_content(content, None, None, &[], &[])
                    .err()
                    .map(|r| r.class),
                Some("absolute_path"),
                "`{content}` was accepted"
            );
        }
    }

    /// Multi-component absolute paths still are, and prose still is not.
    ///
    /// The second half is what keeps the widening honest: an internal slash never
    /// starts a token, so `and/or` was never in this class's scope, and `/` alone
    /// is not a path.
    #[test]
    fn prose_with_an_internal_slash_is_still_accepted() {
        assert_eq!(
            validate_global_content("the fix lives in /Users/dev/main.rs", None, None, &[], &[])
                .err()
                .map(|r| r.class),
            Some("absolute_path")
        );
        for clean in [
            "prefer read/write over readwrite",
            "either and/or is fine",
            "use / as the separator",
            "a ratio of 3/4",
        ] {
            assert!(
                validate_global_content(clean, None, None, &[], &[]).is_ok(),
                "`{clean}` was refused as an absolute path"
            );
        }
    }
}
