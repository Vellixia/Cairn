//! Privacy-first sanitization - the gate every memory passes through before it can be shared.
//!
//! Cairn's "collective knowledge" only works if nothing private leaks. This crate detects secrets
//! and PII (API keys, tokens, private keys, JWTs, emails, IPs, home-directory paths, and
//! high-entropy blobs) with **real parsing rules**, redacts them in place, and classifies the
//! result:
//!
//! - [`Sensitivity::Shareable`] - nothing sensitive found; safe to pool.
//! - [`Sensitivity::NeedsReview`] - contains PII-ish signal (email/IP/home path/high-entropy);
//!   shareable only after a human (or policy) okays it.
//! - [`Sensitivity::Private`] - contains a hard secret (key/token/password); **never** auto-shared.
//!
//! The redaction is conservative on purpose: when in doubt it over-redacts, because the cost of a
//! leaked credential dwarfs the cost of a `[redacted:...]` placeholder.

use cairn_core::{Memory, MemoryKind, NewMemory};
use regex::Regex;
use serde::{Deserialize, Serialize};

/// How sensitive a piece of text is, and therefore how freely it may be shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Shareable,
    NeedsReview,
    Private,
}

impl Sensitivity {
    /// Only `Shareable` is safe to pool without review.
    pub fn is_shareable(self) -> bool {
        matches!(self, Sensitivity::Shareable)
    }

    fn rank(self) -> u8 {
        match self {
            Sensitivity::Shareable => 0,
            Sensitivity::NeedsReview => 1,
            Sensitivity::Private => 2,
        }
    }

    /// The more-sensitive of the two.
    fn escalate(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

/// What kind of sensitive token was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    PrivateKey,
    AwsKey,
    GithubToken,
    SlackToken,
    GoogleApiKey,
    StripeKey,
    OpenAiKey,
    AnthropicKey,
    GenericSecret,
    Jwt,
    NamedSecret,
    BearerToken,
    Email,
    IpAddress,
    HomePath,
    HighEntropy,
}

impl SecretKind {
    /// The placeholder substituted for a finding of this kind.
    pub fn placeholder(self) -> &'static str {
        match self {
            SecretKind::PrivateKey => "[redacted:private_key]",
            SecretKind::AwsKey => "[redacted:aws_key]",
            SecretKind::GithubToken => "[redacted:github_token]",
            SecretKind::SlackToken => "[redacted:slack_token]",
            SecretKind::GoogleApiKey => "[redacted:google_api_key]",
            SecretKind::StripeKey => "[redacted:stripe_key]",
            SecretKind::OpenAiKey => "[redacted:openai_key]",
            SecretKind::AnthropicKey => "[redacted:anthropic_key]",
            SecretKind::GenericSecret => "[redacted:secret]",
            SecretKind::Jwt => "[redacted:jwt]",
            SecretKind::NamedSecret => "[redacted:secret]",
            SecretKind::BearerToken => "[redacted:token]",
            SecretKind::Email => "[redacted:email]",
            SecretKind::IpAddress => "[redacted:ip]",
            SecretKind::HomePath => "[redacted:home_path]",
            SecretKind::HighEntropy => "[redacted:high_entropy]",
        }
    }

    fn sensitivity(self) -> Sensitivity {
        match self {
            SecretKind::Email
            | SecretKind::IpAddress
            | SecretKind::HomePath
            | SecretKind::HighEntropy => Sensitivity::NeedsReview,
            _ => Sensitivity::Private,
        }
    }

    /// Resolution priority when findings overlap: a specific secret beats a generic high-entropy hit.
    fn priority(self) -> u8 {
        match self {
            SecretKind::HighEntropy => 0,
            SecretKind::Email | SecretKind::IpAddress | SecretKind::HomePath => 1,
            _ => 2,
        }
    }
}

/// One detected sensitive span (byte offsets into the scanned text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub kind: SecretKind,
    pub start: usize,
    pub end: usize,
}

/// The result of sanitizing a string.
#[derive(Debug, Clone, Serialize)]
pub struct Sanitized {
    /// The text with every finding replaced by a placeholder.
    pub text: String,
    /// Every finding, ordered by position.
    pub findings: Vec<Finding>,
    /// The overall classification (the most sensitive finding wins).
    pub sensitivity: Sensitivity,
}

/// A memory rewritten so it is safe to consider for sharing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareableMemory {
    pub kind: MemoryKind,
    pub content: String,
    #[serde(default)]
    pub concepts: Vec<String>,
    pub sensitivity: Sensitivity,
    /// How many spans were redacted out of the content.
    #[serde(default)]
    pub redactions: usize,
}

impl ShareableMemory {
    /// Convert an ingested shareable memory into a new local memory, tagged with `shared`
    /// provenance so it stays distinguishable from first-party memories.
    pub fn into_new_memory(self) -> NewMemory {
        let mut nm = NewMemory::new(self.content);
        nm.kind = Some(self.kind);
        nm.concepts = self.concepts;
        nm.session_id = Some("shared".to_string());
        nm
    }
}

/// A portable bundle of sanitized memories - the unit of collective-knowledge exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareBundle {
    pub schema: String,
    pub version: u32,
    pub memories: Vec<ShareableMemory>,
}

impl ShareBundle {
    /// The schema tag stamped on every bundle.
    pub const SCHEMA: &'static str = "cairn-share-bundle";

    /// Convert the bundle's memories into new local memories ready to remember.
    pub fn into_new_memories(self) -> Vec<NewMemory> {
        self.memories
            .into_iter()
            .map(ShareableMemory::into_new_memory)
            .collect()
    }
}

/// Counts from building a [`ShareBundle`].
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ShareStats {
    pub total: usize,
    pub shared: usize,
    pub needs_review: usize,
    pub withheld: usize,
}

struct Pattern {
    kind: SecretKind,
    re: Regex,
    /// Which capture group to redact (0 = the whole match; >0 redacts just the value).
    group: usize,
}

/// The sanitizer. Compiles its rule set once; `scan`/`sanitize` are then cheap and allocation-light.
pub struct Sanitizer {
    patterns: Vec<Pattern>,
    token_re: Regex,
    home_re: Regex,
}

impl Default for Sanitizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Sanitizer {
    pub fn new() -> Self {
        // (kind, regex, capture-group-to-redact). Order matters only for tie-breaking; correctness
        // comes from `priority()` + overlap resolution.
        let specs: &[(SecretKind, &str, usize)] = &[
            (
                SecretKind::PrivateKey,
                r"-----BEGIN (?:[A-Z]+ )?PRIVATE KEY-----[\s\S]*?-----END (?:[A-Z]+ )?PRIVATE KEY-----",
                0,
            ),
            (SecretKind::AnthropicKey, r"sk-ant-[A-Za-z0-9_-]{20,}", 0),
            (SecretKind::OpenAiKey, r"sk-(?:proj-)?[A-Za-z0-9_-]{20,}", 0),
            (SecretKind::GenericSecret, r"sk-[A-Za-z0-9_-]{16,}", 0),
            (SecretKind::AwsKey, r"AKIA[0-9A-Z]{16}", 0),
            (
                SecretKind::GithubToken,
                r"(?:gh[pousr]_[A-Za-z0-9]{36}|github_pat_[0-9A-Za-z_]{40,})",
                0,
            ),
            (SecretKind::SlackToken, r"xox[baprs]-[A-Za-z0-9-]{10,}", 0),
            (SecretKind::GoogleApiKey, r"AIza[0-9A-Za-z_-]{35}", 0),
            (
                SecretKind::StripeKey,
                r"(?:sk|pk|rk)_(?:live|test)_[0-9A-Za-z]{16,}",
                0,
            ),
            (
                SecretKind::Jwt,
                r"eyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
                0,
            ),
            (
                SecretKind::BearerToken,
                r"(?i)(bearer\s+)([A-Za-z0-9._-]{16,})",
                2,
            ),
            (
                SecretKind::NamedSecret,
                r#"(?i)(?:api[_-]?key|client[_-]?secret|access[_-]?key|auth[_-]?token|secret|token|password|passwd|pwd)\s*[:=]\s*["']?([^\s"']{6,})"#,
                1,
            ),
            (
                SecretKind::Email,
                r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}",
                0,
            ),
            (SecretKind::IpAddress, r"\b(?:\d{1,3}\.){3}\d{1,3}\b", 0),
            (
                SecretKind::HomePath,
                r"(?:/(?:home|Users)/[^/\s:]+|[A-Za-z]:\\Users\\[^\\\s:]+)",
                0,
            ),
        ];

        let patterns = specs
            .iter()
            .map(|(kind, src, group)| Pattern {
                kind: *kind,
                re: Regex::new(src).expect("sanitizer pattern must compile"),
                group: *group,
            })
            .collect();

        Self {
            patterns,
            token_re: Regex::new(r"[A-Za-z0-9+/_-]{32,}").unwrap(),
            home_re: Regex::new(r"(?:/(?:home|Users)/[^/\s:]+|[A-Za-z]:\\Users\\[^\\\s:]+)")
                .unwrap(),
        }
    }

    /// Find every sensitive span in `text`, de-overlapped and ordered by position.
    pub fn scan(&self, text: &str) -> Vec<Finding> {
        let mut raw: Vec<Finding> = Vec::new();
        for p in &self.patterns {
            for caps in p.re.captures_iter(text) {
                if let Some(m) = caps.get(p.group) {
                    raw.push(Finding {
                        kind: p.kind,
                        start: m.start(),
                        end: m.end(),
                    });
                }
            }
        }
        // High-entropy blobs that no named rule caught (random keys, hashes-that-are-secrets, ...).
        for m in self.token_re.find_iter(text) {
            if looks_like_secret(m.as_str()) {
                raw.push(Finding {
                    kind: SecretKind::HighEntropy,
                    start: m.start(),
                    end: m.end(),
                });
            }
        }
        resolve_overlaps(raw)
    }

    /// Redact every finding in `text` and classify the result.
    pub fn sanitize(&self, text: &str) -> Sanitized {
        let findings = self.scan(text);
        let mut out = String::with_capacity(text.len());
        let mut sensitivity = Sensitivity::Shareable;
        let mut last = 0usize;
        for f in &findings {
            out.push_str(&text[last..f.start]);
            out.push_str(f.kind.placeholder());
            last = f.end;
            sensitivity = sensitivity.escalate(f.kind.sensitivity());
        }
        out.push_str(&text[last..]);
        Sanitized {
            text: out,
            findings,
            sensitivity,
        }
    }

    /// Rewrite a memory into a shareable form: content + concepts redacted, and a sensitivity that
    /// also accounts for home-revealing file paths.
    pub fn sanitize_memory(&self, m: &Memory) -> ShareableMemory {
        let content = self.sanitize(&m.content);
        let concepts = m.concepts.iter().map(|c| self.sanitize(c).text).collect();
        let mut sensitivity = content.sensitivity;
        if m.files.iter().any(|f| self.home_re.is_match(f)) {
            sensitivity = sensitivity.escalate(Sensitivity::NeedsReview);
        }
        ShareableMemory {
            kind: m.kind,
            content: content.text,
            concepts,
            sensitivity,
            redactions: content.findings.len(),
        }
    }

    /// Build a shareable bundle from local memories: redact each, withhold any that still classify
    /// as private, and report the counts. `NeedsReview` memories are included (the receiver decides).
    pub fn bundle(&self, mems: &[Memory]) -> (ShareBundle, ShareStats) {
        let mut memories = Vec::new();
        let mut stats = ShareStats {
            total: mems.len(),
            shared: 0,
            needs_review: 0,
            withheld: 0,
        };
        for m in mems {
            let sm = self.sanitize_memory(m);
            match sm.sensitivity {
                Sensitivity::Private => stats.withheld += 1,
                Sensitivity::NeedsReview => {
                    stats.needs_review += 1;
                    memories.push(sm);
                }
                Sensitivity::Shareable => memories.push(sm),
            }
        }
        stats.shared = memories.len();
        (
            ShareBundle {
                schema: ShareBundle::SCHEMA.to_string(),
                version: 1,
                memories,
            },
            stats,
        )
    }
}

/// Priority-first non-overlapping selection. A specific secret must win over a generic
/// high-entropy hit *even when the generic one starts earlier* (e.g. `key=sk-ant-...` - the whole
/// thing reads as a high-entropy blob, but the `sk-ant-` key is what matters). So we claim spans in
/// order of specificity (then length, then position), dropping anything that overlaps an
/// already-claimed span, and finally re-sort by position for rebuilding the text.
fn resolve_overlaps(mut v: Vec<Finding>) -> Vec<Finding> {
    v.sort_by(|a, b| {
        b.kind
            .priority()
            .cmp(&a.kind.priority())
            .then((b.end - b.start).cmp(&(a.end - a.start)))
            .then(a.start.cmp(&b.start))
    });
    let mut kept: Vec<Finding> = Vec::new();
    for f in v {
        let overlaps = kept.iter().any(|k| f.start < k.end && k.start < f.end);
        if !overlaps {
            kept.push(f);
        }
    }
    kept.sort_by_key(|f| f.start);
    kept
}

/// A long, space-free token is "secret-like" if it's high-entropy and mixes character classes ---
/// which rules out ordinary prose and identifiers while catching random credentials.
fn looks_like_secret(s: &str) -> bool {
    if s.len() < 32 {
        return false;
    }
    let has_digit = s.bytes().any(|b| b.is_ascii_digit());
    let has_upper = s.bytes().any(|b| b.is_ascii_uppercase());
    let has_special = s
        .bytes()
        .any(|b| matches!(b, b'+' | b'/' | b'_' | b'=' | b'-'));
    has_digit && (has_upper || has_special) && shannon_entropy(s) > 4.0
}

/// Shannon entropy in bits per byte.
fn shannon_entropy(s: &str) -> f64 {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let len = bytes.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn san() -> Sanitizer {
        Sanitizer::new()
    }

    /// The core safety property: after sanitizing, the secret literal must be gone.
    fn assert_redacted(input: &str, secret: &str, expect: Sensitivity) {
        let out = san().sanitize(input);
        assert!(
            !out.text.contains(secret),
            "secret leaked through sanitizer!\n  input: {input}\n  output: {}",
            out.text
        );
        assert_eq!(out.sensitivity, expect, "wrong classification for: {input}");
    }

    #[test]
    fn detects_and_redacts_hard_secrets_as_private() {
        // Fixtures are assembled from fragments so the repository never stores a verbatim
        // credential (which would trip secret-scanning push protection). At runtime they're whole
        // and exercise the real provider detectors.
        let cases = [
            format!("AKIA{}", "IOSFODNN7EXAMPLE"),
            format!("ghp_{}", "0123456789abcdefghijklmnopqrstuvwxyz"),
            format!("sk-ant-{}", "api03-abcdefghijklmnopqrstuvwxyz0123"),
            format!("AIza{}", "SyA1234567890abcdefghijklmno_pqrst-u"),
            format!("sk_{}_{}", "live", "abcdefghijklmnop12345678"),
            format!(
                "{}.{}.{}",
                "eyJhbGciOiJIUzI1NiJ9", "eyJzdWIiOiIxMjM0NSJ9", "SflKxwRJSMeKKF2QT4fwpMeJf36"
            ),
        ];
        for secret in &cases {
            assert_redacted(
                &format!("value: {secret} end"),
                secret,
                Sensitivity::Private,
            );
        }
    }

    #[test]
    fn private_key_block_is_redacted() {
        // Markers assembled from fragments (see note above) so the repo holds no PEM key block.
        let begin = format!("-----BEGIN RSA {}-----", "PRIVATE KEY");
        let end = format!("-----END RSA {}-----", "PRIVATE KEY");
        let pem = format!("{begin}\nMIIBfake0LOLOL/not+a+real+key==\n{end}");
        let out = san().sanitize(&format!("here:\n{pem}\nbye"));
        assert!(!out.text.contains("not+a+real+key"));
        assert!(out.text.contains("[redacted:private_key]"));
        assert_eq!(out.sensitivity, Sensitivity::Private);
    }

    #[test]
    fn named_secret_redacts_the_value_but_keeps_the_key() {
        let out = san().sanitize(r#"config: password = "hunter2password""#);
        assert!(!out.text.contains("hunter2password"));
        assert!(out.text.contains("password")); // the key/label survives
        assert!(out.text.contains("[redacted:secret]"));
        assert_eq!(out.sensitivity, Sensitivity::Private);
    }

    #[test]
    fn pii_is_needs_review_not_private() {
        let out = san().sanitize("ping me at alice.smith@example.com or 203.0.113.42");
        assert!(!out.text.contains("alice.smith@example.com"));
        assert!(!out.text.contains("203.0.113.42"));
        assert_eq!(out.sensitivity, Sensitivity::NeedsReview);
    }

    #[test]
    fn home_paths_are_redacted_but_structure_survives() {
        let out =
            san().sanitize("see /Users/alice/projects/app/main.rs and C:\\Users\\bob\\notes.txt");
        assert!(!out.text.contains("/Users/alice"));
        assert!(!out.text.contains("Users\\bob"));
        assert!(out.text.contains("/projects/app/main.rs")); // suffix preserved
        assert_eq!(out.sensitivity, Sensitivity::NeedsReview);
    }

    #[test]
    fn high_entropy_blob_is_caught_but_prose_is_left_alone() {
        let blob = "Z9x2Qw7Lp0Vt8Rk3Nh6Bf1Ym5Cs4Dg2Ae0Uj7Wq"; // 40 random-ish chars
        let out = san().sanitize(&format!("nonce {blob} sent"));
        assert!(!out.text.contains(blob));
        assert_eq!(out.sensitivity, Sensitivity::NeedsReview);

        let prose = "The quick brown fox jumps over the lazy dog and then keeps running along.";
        let clean = san().sanitize(prose);
        assert_eq!(clean.text, prose);
        assert_eq!(clean.sensitivity, Sensitivity::Shareable);
    }

    #[test]
    fn clean_text_is_shareable_and_sanitize_is_idempotent() {
        let out =
            san().sanitize("Decided to use BM25 ranking for recall; it beat TF-IDF in tests.");
        assert_eq!(out.sensitivity, Sensitivity::Shareable);
        assert!(out.findings.is_empty());

        // Re-sanitizing a sanitized string changes nothing (placeholders aren't secrets).
        let token = format!("key=ghp_{}", "0123456789abcdefghijklmnopqrstuvwxyz");
        let once = san().sanitize(&token);
        assert!(once.text.contains("[redacted:github_token]"));
        let twice = san().sanitize(&once.text);
        assert_eq!(once.text, twice.text);
    }

    #[test]
    fn sanitize_memory_redacts_content_and_flags_home_files() {
        let mut m =
            cairn_core::NewMemory::new("login with password=topsecretvalue today").into_memory();
        m.files = vec!["/home/alice/app/src/main.rs".to_string()];
        m.concepts = vec!["auth".to_string()];

        let sm = san().sanitize_memory(&m);
        assert!(!sm.content.contains("topsecretvalue"));
        assert!(sm.redactions >= 1);
        // password makes it Private regardless of the file path.
        assert_eq!(sm.sensitivity, Sensitivity::Private);

        // A clean memory with a home-revealing file path is only NeedsReview.
        let mut m2 = cairn_core::NewMemory::new("refactored the parser into modules").into_memory();
        m2.files = vec!["/home/alice/app/src/parser.rs".to_string()];
        assert_eq!(
            san().sanitize_memory(&m2).sensitivity,
            Sensitivity::NeedsReview
        );
    }

    #[test]
    fn bundle_withholds_private_round_trips_and_ingests_with_provenance() {
        let clean = cairn_core::NewMemory::new("prefer BM25 ranking for recall").into_memory();
        let secret = cairn_core::NewMemory::new("password = supersecretvalue123").into_memory();
        let (bundle, stats) = san().bundle(&[clean, secret]);
        assert_eq!(stats.total, 2);
        assert_eq!(stats.withheld, 1); // the password memory is held back
        assert_eq!(stats.shared, 1);
        assert_eq!(bundle.schema, ShareBundle::SCHEMA);

        // A bundle serializes and parses back (unknown producer fields are ignored).
        let json = serde_json::to_string(&bundle).unwrap();
        let parsed: ShareBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.memories.len(), 1);

        // Ingesting tags each memory with `shared` provenance.
        let news = parsed.into_new_memories();
        assert_eq!(news.len(), 1);
        assert_eq!(news[0].session_id.as_deref(), Some("shared"));
        assert!(news[0].content.contains("BM25"));
    }
}
