//! Key normalization as an identity function (T016, SC-745, FR-796a–d).
//!
//! A topic key and a value key are not labels. They are **the identity of a
//! claim about a subject** (FR-824), and identity is what decides whether a new
//! candidate reinforces existing knowledge, duplicates it, or contradicts it.
//! Two spellings of one key that do not converge therefore do not merely look
//! untidy — they make Cairn hold two answers for one question and, when the
//! answers differ, report a conflict between a claim and itself.
//!
//! So the corpus below is the test. Fifty-plus groups, each a set of spellings
//! that must all land on one canonical key, and each group's members drawn from
//! the ways a person or a model actually writes the same thing: title case,
//! hyphens, spaces, slashes, doubled separators, stray padding.
//!
//! Two things the corpus deliberately does **not** claim:
//!
//! - **Dots still segment a topic key.** `a.b` is two segments and stays two.
//!   Folding them would merge `deploy.images` into `deployimages` and change
//!   which subject a claim is about.
//! - **Normalization is not repair.** On the strict path a character folding
//!   cannot represent refuses the candidate rather than being dropped, because
//!   dropping it produces a *plausible* key naming something nobody proposed
//!   (FR-796b).

use cairn_core::knowledge::{
    normalize_candidate_keys, normalize_topic_key, normalize_topic_key_strict, normalize_value_key,
    normalize_value_key_strict, KeyRefusal, TOPIC_KEY_MAX_CHARS, TOPIC_KEY_MAX_SEGMENTS,
    VALUE_KEY_MAX_CHARS,
};

/// Groups of spellings that must all resolve to the group's first entry.
///
/// The canonical form is stated explicitly rather than derived, so a change in
/// the folding shows up here as a diff instead of as agreement with itself.
///
/// A group's members are all the *same* shape: either all dotted or all
/// dotless. That is not tidiness. `logging.level` and `logging-level` are two
/// different keys — one subject with two segments, one subject with one — and a
/// group that mixed them would be asserting a folding that must never happen.
const TOPIC_GROUPS: &[(&str, &[&str])] = &[
    (
        "storage_authority",
        &[
            "Storage Authority",
            "storage_authority",
            "storage-authority",
        ],
    ),
    (
        "deploy.images",
        &[
            "Deploy.Images",
            "deploy .images",
            "DEPLOY.images",
            " deploy.images ",
        ],
    ),
    (
        "auth.token_expiry",
        &[
            "Auth.Token Expiry",
            "auth.token-expiry",
            "AUTH.token_expiry",
        ],
    ),
    (
        "database.connection_pool",
        &["Database.Connection Pool", "database.connection-pool"],
    ),
    (
        "ci.windows_runner",
        &[
            "CI.Windows Runner",
            "ci.windows-runner",
            "Ci.Windows/Runner",
        ],
    ),
    (
        "release_tagging",
        &["Release Tagging", "release-tagging", "release/tagging"],
    ),
    ("test.flake", &["Test.Flake", "test .flake", "TEST.FLAKE"]),
    (
        "sync_backoff",
        &["Sync Backoff", "sync--backoff", "sync___backoff"],
    ),
    (
        "http_timeout",
        &["HTTP Timeout", "http-timeout", "Http/Timeout"],
    ),
    (
        "api.rate_limit",
        &["API.Rate Limit", "api.rate-limit", "api.rate/limit"],
    ),
    (
        "cache_eviction",
        &["Cache Eviction", "cache_eviction", "  cache-eviction  "],
    ),
    ("logging_level", &["logging-level", "LOGGING/LEVEL"]),
    (
        "secrets_rotation",
        &["Secrets Rotation", "secrets-rotation", "secrets/rotation"],
    ),
    (
        "build_profile",
        &["Build Profile", "build_profile", "build-profile"],
    ),
    (
        "schema_migration",
        &["Schema Migration", "schema-migration", "schema/migration"],
    ),
    (
        "queue_retry",
        &["Queue Retry", "queue_retry", "Queue--Retry"],
    ),
    (
        "index_strategy",
        &["Index Strategy", "index-strategy", "INDEX_STRATEGY"],
    ),
    (
        "session_lifecycle",
        &["session-lifecycle", "session lifecycle"],
    ),
    (
        "hook_deadline",
        &["Hook Deadline", "hook-deadline", "hook/deadline"],
    ),
    (
        "privacy_redaction",
        &[
            "Privacy Redaction",
            "privacy-redaction",
            "privacy/redaction",
        ],
    ),
    (
        "agent.claude_code",
        &[
            "Agent.Claude Code",
            "agent.claude-code",
            "agent.Claude_Code",
        ],
    ),
    ("agent_codex", &["agent-codex", "AGENT/CODEX"]),
    ("agent_opencode", &["agent-opencode", "agent/openCode"]),
    (
        "server_authority",
        &["Server Authority", "server-authority"],
    ),
    (
        "client_spool",
        &["Client Spool", "client_spool", "client-spool"],
    ),
    (
        "consolidation_lease",
        &["Consolidation Lease", "consolidation-lease"],
    ),
    (
        "retrieval_budget",
        &["Retrieval Budget", "retrieval-budget", "retrieval/budget"],
    ),
    (
        "delivery_dedup",
        &["Delivery Dedup", "delivery-dedup", "DELIVERY_DEDUP"],
    ),
    (
        "verification_authority",
        &["Verification Authority", "verification-authority"],
    ),
    (
        "pattern_promotion",
        &[
            "Pattern Promotion",
            "pattern-promotion",
            "pattern/promotion",
        ],
    ),
    (
        "migration_cutover",
        &[
            "Migration Cutover",
            "migration-cutover",
            "MIGRATION/CUTOVER",
        ],
    ),
    (
        "health_evidence",
        &["Health Evidence", "health-evidence", "health/evidence"],
    ),
    (
        "web_dashboard",
        &["Web Dashboard", "web-dashboard", "WEB/DASHBOARD"],
    ),
    (
        "git_worktree",
        &["Git Worktree", "git-worktree", "git/worktree"],
    ),
    (
        "branch_protection",
        &["Branch Protection", "branch-protection"],
    ),
    (
        "commit_signing",
        &["Commit Signing", "commit-signing", "commit/signing"],
    ),
    (
        "docker_image",
        &["Docker Image", "docker-image", "DOCKER/IMAGE"],
    ),
    ("deploy_rollback", &["deploy-rollback", "deploy/rollback"]),
    (
        "metrics_latency",
        &["Metrics Latency", "metrics-latency", "METRICS_LATENCY"],
    ),
    (
        "tracing_sampling",
        &["Tracing Sampling", "tracing-sampling"],
    ),
    (
        "config_precedence",
        &["Config Precedence", "config-precedence"],
    ),
    ("cli_output", &["CLI Output", "cli-output", "Cli/Output"]),
    (
        "mcp.tool_surface",
        &["MCP.Tool Surface", "mcp.tool-surface", "mcp.TOOL_SURFACE"],
    ),
    (
        "handoff_generation",
        &["Handoff Generation", "handoff-generation"],
    ),
    (
        "checkpoint_turn",
        &["Checkpoint Turn", "checkpoint-turn", "checkpoint/turn"],
    ),
    (
        "context_compaction",
        &["Context Compaction", "context-compaction"],
    ),
    (
        "budget_reserve",
        &["Budget Reserve", "budget-reserve", "BUDGET/RESERVE"],
    ),
    (
        "domain_separation",
        &["Domain Separation", "domain-separation"],
    ),
    (
        "team_ratification",
        &["Team Ratification", "team-ratification"],
    ),
    (
        "personal_ownership",
        &["Personal Ownership", "personal-ownership"],
    ),
    (
        "outbox_claim",
        &["Outbox Claim", "outbox-claim", "outbox/claim"],
    ),
    (
        "event_identity",
        &["Event Identity", "event-identity", "EVENT/IDENTITY"],
    ),
    (
        "extraction_determinism",
        &["Extraction Determinism", "extraction-determinism"],
    ),
    (
        "vocabulary_token",
        &["Vocabulary Token", "vocabulary-token", "vocabulary/token"],
    ),
];

const VALUE_GROUPS: &[(&str, &[&str])] = &[
    ("server", &["Server", "SERVER", "  server  "]),
    (
        "server_authoritative",
        &[
            "Server Authoritative",
            "server-authoritative",
            "SERVER_AUTHORITATIVE",
        ],
    ),
    (
        "postgresql_16",
        &["PostgreSQL 16", "postgresql-16", "PostgreSQL\t16"],
    ),
    ("read_write", &["Read Write", "read-write", "read/write"]),
    (
        "five_minutes",
        &["Five Minutes", "five-minutes", "FIVE_MINUTES"],
    ),
    ("owner_only", &["Owner Only", "owner-only", "Owner/Only"]),
    ("remote_attested", &["Remote Attested", "remote-attested"]),
    (
        "pre_cutover",
        &["Pre Cutover", "pre-cutover", "PRE_CUTOVER"],
    ),
    (
        "no_evidence",
        &["No Evidence", "no-evidence", "NO/EVIDENCE"],
    ),
    ("deterministic", &["Deterministic", "  DETERMINISTIC  "]),
    // The dot is content in a value key, not a separator: these are versions.
    ("1.2.3", &["1.2.3", " 1.2.3 "]),
    ("v0.1.0_alpha.5", &["v0.1.0-alpha.5", "V0.1.0 alpha.5"]),
];

#[test]
fn every_spelling_of_a_topic_key_resolves_to_one_canonical_form() {
    assert!(
        TOPIC_GROUPS.len() >= 50,
        "SC-745 asks for at least fifty groups; this corpus has {}",
        TOPIC_GROUPS.len()
    );
    // Some groups are multi-segment on purpose. A corpus of single-segment
    // keys would pass just as well against a normalizer that had stopped
    // treating the dot as a separator, which is the one change here that
    // silently merges two subjects.
    assert!(
        TOPIC_GROUPS
            .iter()
            .filter(|(canonical, _)| canonical.contains('.'))
            .count()
            >= 5,
        "the corpus lost its multi-segment coverage"
    );
    for (canonical, spellings) in TOPIC_GROUPS {
        for spelling in *spellings {
            assert_eq!(
                normalize_topic_key(spelling).as_deref(),
                Some(*canonical),
                "{spelling:?} did not resolve to {canonical:?}"
            );
            assert_eq!(
                normalize_topic_key_strict(spelling).ok().as_deref(),
                Some(*canonical),
                "{spelling:?} did not resolve to {canonical:?} on the strict path"
            );
        }
        // The canonical form is a fixed point. A normalizer that changed its
        // own output would make identity depend on how many times it ran.
        assert_eq!(
            normalize_topic_key(canonical).as_deref(),
            Some(*canonical),
            "{canonical:?} is not a fixed point"
        );
    }
}

#[test]
fn every_spelling_of_a_value_key_resolves_to_one_canonical_form() {
    for (canonical, spellings) in VALUE_GROUPS {
        for spelling in *spellings {
            assert_eq!(
                normalize_value_key(spelling).as_deref(),
                Some(*canonical),
                "{spelling:?} did not resolve to {canonical:?}"
            );
            assert_eq!(
                normalize_value_key_strict(spelling).ok().as_deref(),
                Some(*canonical)
            );
        }
        assert_eq!(normalize_value_key(canonical).as_deref(), Some(*canonical));
    }
}

#[test]
fn the_lenient_and_strict_paths_agree_wherever_both_succeed() {
    // They differ only in what they do with input neither can represent. Where
    // both produce a key, producing *different* keys would mean a memory and
    // the candidate that restates it disagreed about their own subject.
    for (_, spellings) in TOPIC_GROUPS.iter().chain(VALUE_GROUPS.iter()) {
        for spelling in *spellings {
            if let (Some(lenient), Ok(strict)) = (
                normalize_topic_key(spelling),
                normalize_topic_key_strict(spelling),
            ) {
                assert_eq!(lenient, strict, "the two paths disagree on {spelling:?}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dot segmentation is unchanged (SC-745)
// ---------------------------------------------------------------------------

#[test]
fn a_dot_still_separates_topic_segments_and_still_does_not_in_a_value() {
    assert_eq!(
        normalize_topic_key("deploy.images").as_deref(),
        Some("deploy.images"),
        "the dot stopped separating, so two subjects merged into one"
    );
    assert_eq!(normalize_topic_key("a..b").as_deref(), Some("a.b"));
    assert_eq!(normalize_topic_key(".a.").as_deref(), Some("a"));
    assert_eq!(
        normalize_topic_key("a.b.c.d.e.f").as_deref(),
        Some("a.b.c.d.e.f")
    );
    // Seven segments is one too many, and is refused rather than truncated to
    // six — a truncated key names a different subject.
    assert_eq!(normalize_topic_key("a.b.c.d.e.f.g"), None);
    assert_eq!(
        normalize_topic_key_strict("a.b.c.d.e.f.g"),
        Err(KeyRefusal::TooManySegments {
            max: TOPIC_KEY_MAX_SEGMENTS
        })
    );

    // In a value key the dot is content, so a version is not three segments.
    assert_eq!(normalize_value_key("1.2.3").as_deref(), Some("1.2.3"));
    assert_ne!(normalize_value_key("1.2.3").as_deref(), Some("123"));
}

#[test]
fn a_slash_folds_rather_than_segmenting_so_a_key_cannot_become_a_path() {
    // A topic key is not a path, and accepting path syntax would invite an
    // absolute path into a column that synchronizes.
    assert_eq!(
        normalize_topic_key("crates/cairn-core").as_deref(),
        Some("crates_cairn_core")
    );
    assert_eq!(
        normalize_topic_key("/etc/passwd").as_deref(),
        Some("etc_passwd")
    );
}

// ---------------------------------------------------------------------------
// Refusal, not repair (FR-796b)
// ---------------------------------------------------------------------------

#[test]
fn a_character_folding_cannot_represent_refuses_the_key_rather_than_vanishing() {
    for (input, repaired_to) in [
        ("storage@authority", "storageauthority"),
        ("deploy!images", "deployimages"),
        ("auth(token)", "authtoken"),
        ("a+b", "ab"),
        ("café.setting", "caf.setting"),
        ("emoji🙂key", "emojikey"),
        ("key;drop", "keydrop"),
        ("100%", "100"),
    ] {
        // The lenient path repairs, because FR-312 stores the memory either
        // way and the key is a convenience there.
        assert_eq!(
            normalize_topic_key(input).as_deref(),
            Some(repaired_to),
            "the lenient path changed behaviour for {input:?}"
        );
        // The strict path must not, because here the key *is* the identity: a
        // repaired key silently changes which existing knowledge the candidate
        // collides with.
        assert_eq!(
            normalize_topic_key_strict(input),
            Err(KeyRefusal::UnrepresentableCharacter),
            "{input:?} was silently repaired into {repaired_to:?} on the strict path"
        );
    }
}

#[test]
fn an_empty_or_over_long_key_is_refused_with_the_reason_that_applies() {
    assert_eq!(normalize_topic_key_strict(""), Err(KeyRefusal::Empty));
    assert_eq!(normalize_topic_key_strict("   "), Err(KeyRefusal::Empty));
    assert_eq!(normalize_topic_key_strict("..."), Err(KeyRefusal::Empty));
    assert_eq!(normalize_value_key_strict(""), Err(KeyRefusal::Empty));

    let long_topic = "a".repeat(TOPIC_KEY_MAX_CHARS + 1);
    assert_eq!(
        normalize_topic_key_strict(&long_topic),
        Err(KeyRefusal::TooLong {
            max: TOPIC_KEY_MAX_CHARS
        })
    );
    assert!(normalize_topic_key_strict(&"a".repeat(TOPIC_KEY_MAX_CHARS)).is_ok());

    let long_value = "v".repeat(VALUE_KEY_MAX_CHARS + 1);
    assert_eq!(
        normalize_value_key_strict(&long_value),
        Err(KeyRefusal::TooLong {
            max: VALUE_KEY_MAX_CHARS
        })
    );
    assert!(normalize_value_key_strict(&"v".repeat(VALUE_KEY_MAX_CHARS)).is_ok());
}

#[test]
fn a_refusal_carries_no_text_from_the_key_that_caused_it() {
    let err = normalize_topic_key_strict("password=hunter2hunter2@example")
        .expect_err("this key is unrepresentable");
    for rendering in [format!("{err}"), format!("{err:?}")] {
        assert!(!rendering.contains("hunter2"), "a refusal leaked its input");
        assert!(!rendering.contains("password"));
    }
    assert_eq!(err.reason(), "key_normalization_failed");
}

#[test]
fn a_value_key_without_a_topic_key_names_a_value_of_nothing() {
    assert_eq!(
        normalize_candidate_keys(None, Some("server")),
        Err(KeyRefusal::ValueWithoutTopic)
    );
    assert_eq!(normalize_candidate_keys(None, None), Ok((None, None)));
    assert_eq!(
        normalize_candidate_keys(Some("Storage Authority"), None),
        Ok((Some("storage_authority".into()), None))
    );
    assert_eq!(
        normalize_candidate_keys(Some("Storage Authority"), Some("Server Authoritative")),
        Ok((
            Some("storage_authority".into()),
            Some("server_authoritative".into())
        ))
    );
    // A bad value refuses the pair, rather than the candidate being stored with
    // a topic and a quietly-dropped value.
    assert!(normalize_candidate_keys(Some("storage.authority"), Some("a@b")).is_err());
}

#[test]
fn normalization_needs_no_model_and_two_different_keys_stay_different() {
    // FR-796c: a deterministic syntactic function. Two keys are the same key or
    // they are different keys; there is no similarity in between, which is what
    // lets reconciliation rest on this without inference.
    let pairs = [
        ("storage_authority", "storage_authorities"),
        ("deploy.images", "deploy.image"),
        ("auth.token", "auth.tokens"),
        ("colour", "color"),
        ("db", "database"),
        ("1.2.3", "1.2.4"),
    ];
    for (a, b) in pairs {
        assert_ne!(
            normalize_topic_key(a),
            normalize_topic_key(b),
            "{a:?} and {b:?} were treated as one key"
        );
    }
}
