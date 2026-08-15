//! Verifier execution — the only place Cairn touches the worktree to establish
//! a claim (`contracts/evidence-verification.md` §The verifier catalog).
//!
//! Cairn **executes nothing**. It reads files inside the worktree subject to
//! privacy exclusions, it reads Git through `cairn-git`, and it reads the
//! outcome of a test or command that Feature 001's hooks already *captured*.
//! It does not run a build, a test suite or a shell command, and it does not
//! reach the network (FR-365).
//!
//! That last distinction is what resolves the apparent tension between "a known
//! test result at a commit is valid verification" and "no autonomous command
//! execution": the hook recorded the command, the exit code and the commit, so
//! Cairn observed the result without running anything (D52).

use crate::state::Daemon;
use cairn_core::config::CairnConfig;
use cairn_core::domain::{
    EvidenceCollector, VerifierKind, VerifyResult, VerifyTrigger,
};
use cairn_core::verify::{fingerprint, Observed};
use cairn_store::evidence::{self, EvidenceFact, NewRun};
use std::path::Path;
use uuid::Uuid;

/// What one verifier run found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub result: VerifyResult,
    /// The fingerprint as it is now. `None` when the check could not look.
    pub observed: Option<String>,
    /// Bounded and redacted. Why inconclusive, or what differed.
    pub detail: Option<String>,
}

impl Outcome {
    fn inconclusive(detail: impl Into<String>) -> Self {
        Self {
            result: VerifyResult::Inconclusive,
            observed: None,
            detail: Some(detail.into()),
        }
    }
}

/// Run the verifier a fact's kind implies, against the current worktree.
///
/// The result is compared to the fact's recorded fingerprint: equal is
/// `verified`, different is `drifted`, and unreadable is `inconclusive` — the
/// memory then becomes neither (FR-366).
///
/// Attested evidence is **never re-collected**. A recheck of an attested fact
/// is inconclusive until the agent attests again, which is what stops attested
/// evidence decaying into a permanent unfalsifiable claim.
pub fn run_verifier(
    worktree: &Path,
    config: &CairnConfig,
    fact: &EvidenceFact,
    verifier: VerifierKind,
    captured: Option<&CapturedOutcome>,
) -> Outcome {
    if fact.deleted {
        return Outcome::inconclusive("the evidence fact was deleted");
    }
    let Some(locator) = fact.source_locator.as_deref() else {
        return Outcome::inconclusive("the evidence fact has no locator");
    };

    // A path Cairn was told not to look at is `evidence_excluded`, which is
    // distinguishable from `no_evidence`: "I was told not to look" and "nobody
    // attached anything" are different answers.
    if config.is_path_excluded(locator) {
        return Outcome::inconclusive("evidence_excluded: the locator matches an exclusion");
    }

    if fact.collector == EvidenceCollector::Agent {
        // Cairn does not re-run an agent's observation. It has no way to.
        return match captured {
            Some(c) => attested_outcome(fact, verifier, c),
            None => Outcome::inconclusive(
                "attested evidence is not re-collected; the agent must attest again",
            ),
        };
    }

    match verifier {
        VerifierKind::FileExists => file_exists(worktree, locator, fact),
        VerifierKind::FileDigest => file_digest(worktree, locator, fact, config),
        VerifierKind::GitRef | VerifierKind::GitCommit => git_object(worktree, locator, fact),
        VerifierKind::Configuration | VerifierKind::SchemaVersion => {
            configuration(worktree, locator, fact, config)
        }
        VerifierKind::TestOutcome | VerifierKind::CommandOutcome => match captured {
            Some(c) => attested_outcome(fact, verifier, c),
            None => Outcome::inconclusive(
                "no captured observation matches at or after the claimed commit",
            ),
        },
        VerifierKind::RuntimeState => Outcome::inconclusive(
            "runtime state is attested by an agent, never read by Cairn",
        ),
    }
}

/// A test or command outcome Feature 001's hooks already recorded.
#[derive(Debug, Clone)]
pub struct CapturedOutcome {
    pub outcome: String,
    pub exit_code: i64,
    pub commit: Option<String>,
}

fn compare(fact: &EvidenceFact, observed: String, what: &str) -> Outcome {
    match fact.fingerprint.as_deref() {
        Some(recorded) if recorded == observed => Outcome {
            result: VerifyResult::Verified,
            observed: Some(observed),
            detail: None,
        },
        Some(_) => Outcome {
            result: VerifyResult::Drifted,
            observed: Some(observed),
            detail: Some(format!("{what} differs from what was recorded")),
        },
        None => Outcome::inconclusive("the evidence fact has no recorded fingerprint"),
    }
}

/// The one place a locator becomes a filesystem path.
///
/// Refuses anything that escapes the worktree even after the store's own
/// validation, because a locator can arrive from an import that predates it.
fn resolve(worktree: &Path, locator: &str) -> Option<std::path::PathBuf> {
    let relative = locator.split('#').next().unwrap_or(locator);
    if cairn_store::evidence::validate_locator(relative).is_err() {
        return None;
    }
    let joined = worktree.join(relative);
    // A symlink out of the tree is still out of the tree.
    let canonical = joined.canonicalize().ok()?;
    let root = worktree.canonicalize().ok()?;
    canonical.starts_with(&root).then_some(canonical)
}

fn file_exists(worktree: &Path, locator: &str, fact: &EvidenceFact) -> Outcome {
    let relative = locator.split('#').next().unwrap_or(locator);
    if cairn_store::evidence::validate_locator(relative).is_err() {
        return Outcome::inconclusive("evidence_outside_worktree");
    }
    let path = worktree.join(relative);
    let (exists, size) = match std::fs::metadata(&path) {
        Ok(m) => (true, m.len()),
        Err(_) => (false, 0),
    };
    // Absence is a *result*, not a failure to look: `exists:0:0` is a
    // legitimate fingerprint and a file that has gone has drifted.
    let observed = fingerprint(
        VerifierKind::FileExists,
        &Observed::FileExistence { exists, size },
    )
    .unwrap_or_default();
    compare(fact, observed, "file presence")
}

fn file_digest(
    worktree: &Path,
    locator: &str,
    fact: &EvidenceFact,
    config: &CairnConfig,
) -> Outcome {
    let Some(path) = resolve(worktree, locator) else {
        return Outcome::inconclusive("evidence_outside_worktree");
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return Outcome::inconclusive("the file could not be read");
    };
    if meta.len() as usize > config.payload_cap_bytes {
        // Reading it would be unbounded work on a path that is meant to be
        // cheap. Saying so beats guessing.
        return Outcome::inconclusive("the file exceeds the payload cap");
    }
    let Ok(bytes) = std::fs::read(&path) else {
        return Outcome::inconclusive("the file could not be read");
    };
    let observed = cairn_core::digest(&String::from_utf8_lossy(&bytes));
    compare(fact, observed, "the file digest")
}

fn git_object(worktree: &Path, locator: &str, fact: &EvidenceFact) -> Outcome {
    match cairn_git::resolve_ref(worktree, locator) {
        Ok(Some(id)) => compare(fact, id, "the resolved object"),
        // An unresolvable ref means the check could not conclude, not that the
        // claim is false (FR-366).
        Ok(None) => Outcome::inconclusive("the ref does not resolve in this clone"),
        Err(_) => Outcome::inconclusive("git could not be read"),
    }
}

/// Read one key from a repository configuration file.
///
/// The locator carries the key after a `#`: `config/app.yml#server.port`. The
/// reader is deliberately small — `key: value`, `key = value` and
/// `"key": value` — because it exists to check a value Cairn was told about,
/// not to be a configuration language.
fn configuration(
    worktree: &Path,
    locator: &str,
    fact: &EvidenceFact,
    config: &CairnConfig,
) -> Outcome {
    let Some((_, key)) = locator.split_once('#') else {
        return Outcome::inconclusive("the locator names no key");
    };
    let Some(path) = resolve(worktree, locator) else {
        return Outcome::inconclusive("evidence_outside_worktree");
    };
    let Ok(meta) = std::fs::metadata(&path) else {
        return Outcome::inconclusive("the file could not be read");
    };
    if meta.len() as usize > config.payload_cap_bytes {
        return Outcome::inconclusive("the file exceeds the payload cap");
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Outcome::inconclusive("the file could not be read");
    };

    match read_key(&text, key) {
        Some(value) => compare(fact, cairn_core::digest(&value), "the configuration value"),
        None => Outcome::inconclusive("the key is absent from the file"),
    }
}

/// Find `key`'s value in a small configuration file.
///
/// A dotted key matches on its last segment, so `server.port` finds `port:` in
/// a nested document without the reader needing to understand nesting. That is
/// a deliberate simplification: it can find the wrong `port` in a file with
/// two, which makes the check *inconclusive-prone* rather than wrong — and a
/// wrong verification is the failure that matters.
pub fn read_key(text: &str, key: &str) -> Option<String> {
    let leaf = key.rsplit('.').next().unwrap_or(key);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let (name, value) = match line.split_once(['=', ':']) {
            Some(pair) => pair,
            None => continue,
        };
        // A JSON line arrives as `{"port": "8080"}` or `  "port": "8080",`, so
        // the structural characters come off both ends before comparing.
        let name = name.trim().trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, '{' | '[' | ',' | '"' | '\'' | '-')
        });
        if name != leaf {
            continue;
        }
        let value = value
            .trim()
            .trim_end_matches(|c: char| {
                c.is_whitespace() || matches!(c, '}' | ']' | ',' | ';')
            })
            .trim()
            .trim_matches(['"', '\''])
            .to_string();
        if value.is_empty() {
            continue;
        }
        return Some(value);
    }
    None
}

fn attested_outcome(
    fact: &EvidenceFact,
    verifier: VerifierKind,
    captured: &CapturedOutcome,
) -> Outcome {
    let observed = match verifier {
        VerifierKind::TestOutcome => fingerprint(
            VerifierKind::TestOutcome,
            &Observed::TestOutcome {
                outcome: captured.outcome.clone(),
                exit_code: captured.exit_code,
                commit: captured.commit.clone(),
            },
        ),
        VerifierKind::CommandOutcome => fingerprint(
            VerifierKind::CommandOutcome,
            &Observed::CommandOutcome {
                exit_code: captured.exit_code,
                commit: captured.commit.clone(),
            },
        ),
        VerifierKind::RuntimeState => Some(cairn_core::digest(&captured.outcome)),
        _ => None,
    };
    match observed {
        Some(o) => compare(fact, o, "the recorded outcome"),
        None => Outcome::inconclusive("no fingerprint form for that verifier"),
    }
}

/// The verifier a fact's kind implies.
pub fn verifier_for(fact: &EvidenceFact) -> Option<VerifierKind> {
    use cairn_core::domain::EvidenceKind as K;
    Some(match fact.kind {
        K::File => VerifierKind::FileDigest,
        K::GitRef => VerifierKind::GitRef,
        K::Configuration => VerifierKind::Configuration,
        K::SchemaVersion => VerifierKind::SchemaVersion,
        K::TestOutcome => VerifierKind::TestOutcome,
        K::CommandOutcome => VerifierKind::CommandOutcome,
        K::RuntimeState => VerifierKind::RuntimeState,
        // An observation is provenance, not a checkable claim of its own.
        K::Observation => return None,
    })
}

/// What one bounded pass did, for `verify_pass_yielded` and for the metrics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassReport {
    pub facts_examined: usize,
    pub runs_recorded: usize,
    pub memories_updated: usize,
    /// True when a cap bound before the work ran out. The remaining work is
    /// picked up next tick; it is never an error (FR-472).
    pub yielded: bool,
}

/// The bounded verification pass, run on the existing 15-minute maintenance
/// tick.
///
/// Order: `needs_recheck` first, oldest `last_verified_at` first, pinned before
/// unpinned, `project` scope before narrower. Capped by
/// `verify_pass_evidence_max`, `verify_pass_runs_max` and
/// `verify_pass_wall_ms`, with concurrency 1. Exceeding any cap yields.
///
/// **No scheduler is introduced.** This joins the tick that already reaps idle
/// sessions, sweeps owed handoffs and marks stale scopes.
pub async fn bounded_pass(d: &Daemon, project_id: Uuid, worktree: &Path) -> PassReport {
    let config = d.config.read().await.clone();
    let started = std::time::Instant::now();
    let mut report = PassReport::default();

    let candidates = match candidates_for_pass(d, project_id, config.verify_pass_evidence_max).await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "verification pass could not read candidates");
            return report;
        }
    };

    for (memory_id, fact_id) in candidates {
        if report.runs_recorded >= config.verify_pass_runs_max
            || started.elapsed().as_millis() as u64 >= config.verify_pass_wall_ms
        {
            report.yielded = true;
            break;
        }

        let Ok(fact) = evidence::fact(&d.store, fact_id).await else {
            continue;
        };
        report.facts_examined += 1;
        let Some(verifier) = verifier_for(&fact) else {
            continue;
        };

        let captured = captured_for(d, &fact).await;
        let outcome = run_verifier(worktree, &config, &fact, verifier, captured.as_ref());

        let branch = cairn_git::status(worktree)
            .map(|s| s.branch)
            .unwrap_or_else(|_| "unknown".into());
        let commit = fact.repo_commit.clone();

        if evidence::record_run(
            &d.store,
            NewRun {
                project_id,
                memory_id: Some(memory_id),
                criterion_id: None,
                verifier,
                evidence_id: Some(fact.id),
                expected_digest: fact.fingerprint.as_deref(),
                observed_digest: outcome.observed.as_deref(),
                result: outcome.result,
                detail: outcome.detail.as_deref(),
                repo_branch: &branch,
                repo_commit: commit.as_deref(),
                trigger: VerifyTrigger::BackgroundPass,
            },
        )
        .await
        .is_ok()
        {
            report.runs_recorded += 1;
            if evidence::rebuild_verification(&d.store, memory_id).await.is_ok() {
                report.memories_updated += 1;
            }
        }
    }

    if report.facts_examined >= config.verify_pass_evidence_max {
        report.yielded = true;
    }
    report
}

/// The memories owed a check, in the documented order.
async fn candidates_for_pass(
    d: &Daemon,
    project_id: Uuid,
    cap: usize,
) -> cairn_store::Result<Vec<(Uuid, Uuid)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT m.id, l.evidence_id
           FROM memories m
           JOIN memory_evidence_facts l ON l.memory_id = m.id AND l.role = 'supports'
           JOIN evidence_facts f ON f.id = l.evidence_id AND f.deleted_at IS NULL
          WHERE m.project_id = ?1 AND m.deleted_at IS NULL
            AND m.verification IN ('needs_recheck', 'unverified', 'drifted')
          ORDER BY CASE m.verification WHEN 'needs_recheck' THEN 0 ELSE 1 END,
                   COALESCE(m.last_verified_at, '') ASC,
                   m.pinned DESC,
                   CASE m.scope WHEN 'project' THEN 0 WHEN 'branch' THEN 1
                                WHEN 'task' THEN 2 ELSE 3 END,
                   m.id
          LIMIT ?2",
    )
    .bind(project_id.to_string())
    .bind(cap as i64)
    .fetch_all(d.store.pool())
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(m, e)| Some((Uuid::parse_str(&m).ok()?, Uuid::parse_str(&e).ok()?)))
        .collect())
}

/// The captured observation behind a test or command outcome, if there is one.
async fn captured_for(d: &Daemon, fact: &EvidenceFact) -> Option<CapturedOutcome> {
    let observation_id = fact.observation_id?;
    let row: Option<(Option<String>, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT outcome, exit_code, commit_sha FROM observations
          WHERE id = ?1 AND deleted_at IS NULL",
    )
    .bind(observation_id.to_string())
    .fetch_optional(d.store.pool())
    .await
    .ok()
    .flatten();

    let (outcome, exit_code, commit) = row?;
    Some(CapturedOutcome {
        outcome: outcome.unwrap_or_default(),
        exit_code: exit_code.unwrap_or(-1),
        commit,
    })
}

/// Run one bounded pass across every project that has a readable worktree.
///
/// The caps in [`bounded_pass`] are **per project**, and the whole sweep is
/// additionally bounded by the number of projects a store holds — which is
/// small, and which is the same set the stale-scope sweep already walks on this
/// tick.
///
/// A project whose worktree has gone is skipped rather than reported: a
/// developer who deleted a checkout has not created an error.
pub async fn sweep_projects(d: &Daemon) -> PassReport {
    let mut total = PassReport::default();

    let projects: Vec<(String, String)> = match sqlx::query_as(
        "SELECT id, git_common_dir FROM projects WHERE deleted_at IS NULL",
    )
    .fetch_all(d.store.pool())
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "verification sweep could not list projects");
            return total;
        }
    };

    for (id, git_common_dir) in projects {
        let Ok(project_id) = Uuid::parse_str(&id) else {
            continue;
        };
        // `git_common_dir` points at `.git`; the worktree is its parent.
        let worktree = Path::new(&git_common_dir)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| Path::new(&git_common_dir).to_path_buf());
        if !worktree.is_dir() {
            continue;
        }

        let report = bounded_pass(d, project_id, &worktree).await;
        total.facts_examined += report.facts_examined;
        total.runs_recorded += report.runs_recorded;
        total.memories_updated += report.memories_updated;
        total.yielded |= report.yielded;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::domain::EvidenceKind;

    fn fact(kind: EvidenceKind, collector: EvidenceCollector, locator: &str, fp: &str)
        -> EvidenceFact
    {
        EvidenceFact {
            id: Uuid::now_v7(),
            project_id: Uuid::now_v7(),
            kind,
            collector,
            subject: "subject".into(),
            observed_value: Some("value".into()),
            value_digest: Some("digest".into()),
            source_locator: Some(locator.into()),
            fingerprint: Some(fp.into()),
            observation_id: None,
            repo_branch: "main".into(),
            repo_commit: None,
            collected_by_session: Uuid::now_v7(),
            deleted: false,
        }
    }

    fn worktree() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("dir");
        std::fs::write(d.path().join("app.yml"), "server:\n  port: 8080\n").expect("write");
        std::fs::create_dir_all(d.path().join("config")).expect("mkdir");
        std::fs::write(d.path().join("config/database.yml"), "backend: postgresql\n")
            .expect("write");
        d
    }

    #[test]
    fn a_file_digest_verifies_and_then_drifts() {
        let d = worktree();
        let digest = cairn_core::digest("server:\n  port: 8080\n");
        let f = fact(EvidenceKind::File, EvidenceCollector::Cairn, "app.yml", &digest);
        let c = CairnConfig::default();

        let out = run_verifier(d.path(), &c, &f, VerifierKind::FileDigest, None);
        assert_eq!(out.result, VerifyResult::Verified);

        std::fs::write(d.path().join("app.yml"), "server:\n  port: 9000\n").expect("write");
        let out = run_verifier(d.path(), &c, &f, VerifierKind::FileDigest, None);
        assert_eq!(out.result, VerifyResult::Drifted);
        assert!(out.observed.is_some(), "drift reports what it found");
    }

    #[test]
    fn an_unreadable_target_is_inconclusive_not_drifted() {
        // FR-366: the memory becomes neither verified nor drifted.
        let d = worktree();
        let f = fact(EvidenceKind::File, EvidenceCollector::Cairn, "missing.yml", "aaa");
        let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::FileDigest, None);
        assert_eq!(out.result, VerifyResult::Inconclusive);
        assert!(out.observed.is_none());
    }

    #[test]
    fn an_excluded_path_is_reported_as_excluded_rather_than_read() {
        let d = worktree();
        std::fs::create_dir_all(d.path().join("secrets")).expect("mkdir");
        std::fs::write(d.path().join("secrets/prod.env"), "API_KEY=xyz\n").expect("write");
        let c = CairnConfig {
            excluded_paths: vec!["secrets/**".into()],
            ..Default::default()
        };
        let f = fact(
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "secrets/prod.env#API_KEY",
            "aaa",
        );
        let out = run_verifier(d.path(), &c, &f, VerifierKind::Configuration, None);
        assert_eq!(out.result, VerifyResult::Inconclusive);
        assert!(
            out.detail.unwrap_or_default().contains("evidence_excluded"),
            "the reason must be distinguishable from no_evidence"
        );
    }

    #[test]
    fn a_locator_escaping_the_worktree_is_refused() {
        let d = worktree();
        for escaping in ["../outside.yml", "/etc/passwd"] {
            let f = fact(EvidenceKind::File, EvidenceCollector::Cairn, escaping, "aaa");
            let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::FileDigest, None);
            assert_eq!(out.result, VerifyResult::Inconclusive, "{escaping}");
        }
    }

    #[test]
    fn a_configuration_value_is_read_by_key() {
        let d = worktree();
        let digest = cairn_core::digest("postgresql");
        let f = fact(
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "config/database.yml#backend",
            &digest,
        );
        let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::Configuration, None);
        assert_eq!(out.result, VerifyResult::Verified);
    }

    #[test]
    fn an_absent_key_is_inconclusive() {
        let d = worktree();
        let f = fact(
            EvidenceKind::Configuration,
            EvidenceCollector::Cairn,
            "config/database.yml#nothing_here",
            "aaa",
        );
        let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::Configuration, None);
        assert_eq!(out.result, VerifyResult::Inconclusive);
    }

    #[test]
    fn the_configuration_reader_handles_the_three_shapes() {
        assert_eq!(read_key("port: 8080\n", "port").as_deref(), Some("8080"));
        assert_eq!(read_key("port = 8080\n", "port").as_deref(), Some("8080"));
        assert_eq!(
            read_key("{\"port\": \"8080\"}\n", "port").as_deref(),
            Some("8080")
        );
        assert_eq!(
            read_key("server:\n  port: 8080\n", "server.port").as_deref(),
            Some("8080"),
            "a dotted key matches on its last segment"
        );
        assert_eq!(read_key("# port: 8080\n", "port"), None, "a comment is not a value");
        assert_eq!(read_key("port:\n", "port"), None, "an empty value is not a value");
    }

    #[test]
    fn attested_evidence_is_never_re_collected() {
        // The rule that stops attested evidence decaying into a permanent
        // unfalsifiable claim.
        let d = worktree();
        let f = fact(
            EvidenceKind::RuntimeState,
            EvidenceCollector::Agent,
            "runtime/health",
            "aaa",
        );
        let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::RuntimeState, None);
        assert_eq!(out.result, VerifyResult::Inconclusive);
        assert!(
            out.detail
                .unwrap_or_default()
                .contains("must attest again"),
            "the reason should say what would resolve it"
        );
    }

    #[test]
    fn a_captured_outcome_verifies_without_running_anything() {
        // D52: the hook recorded the command, the exit code and the commit, so
        // Cairn observed the result without executing it.
        let d = worktree();
        let fp = fingerprint(
            VerifierKind::TestOutcome,
            &Observed::TestOutcome {
                outcome: "passed".into(),
                exit_code: 0,
                commit: Some("abc123".into()),
            },
        )
        .expect("fingerprint");
        let f = fact(EvidenceKind::TestOutcome, EvidenceCollector::Cairn, "cargo test", &fp);
        let captured = CapturedOutcome {
            outcome: "passed".into(),
            exit_code: 0,
            commit: Some("abc123".into()),
        };
        let out = run_verifier(
            d.path(),
            &CairnConfig::default(),
            &f,
            VerifierKind::TestOutcome,
            Some(&captured),
        );
        assert_eq!(out.result, VerifyResult::Verified);

        // The same outcome at a different commit is a different fact.
        let elsewhere = CapturedOutcome {
            commit: Some("def456".into()),
            ..captured
        };
        let out = run_verifier(
            d.path(),
            &CairnConfig::default(),
            &f,
            VerifierKind::TestOutcome,
            Some(&elsewhere),
        );
        assert_eq!(out.result, VerifyResult::Drifted);
    }

    #[test]
    fn a_test_outcome_with_no_captured_observation_is_inconclusive() {
        let d = worktree();
        let f = fact(EvidenceKind::TestOutcome, EvidenceCollector::Cairn, "cargo test", "aaa");
        let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::TestOutcome, None);
        assert_eq!(out.result, VerifyResult::Inconclusive);
    }

    #[test]
    fn a_deleted_fact_verifies_nothing() {
        let d = worktree();
        let mut f = fact(EvidenceKind::File, EvidenceCollector::Cairn, "app.yml", "aaa");
        f.deleted = true;
        let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::FileDigest, None);
        assert_eq!(out.result, VerifyResult::Inconclusive);
    }

    #[test]
    fn a_file_over_the_payload_cap_is_inconclusive_rather_than_read() {
        let d = worktree();
        std::fs::write(d.path().join("big.bin"), "x".repeat(10_000)).expect("write");
        let c = CairnConfig {
            payload_cap_bytes: 1024,
            ..Default::default()
        };
        let f = fact(EvidenceKind::File, EvidenceCollector::Cairn, "big.bin", "aaa");
        let out = run_verifier(d.path(), &c, &f, VerifierKind::FileDigest, None);
        assert_eq!(out.result, VerifyResult::Inconclusive);
    }

    #[test]
    fn file_presence_is_a_result_and_absence_is_drift() {
        let d = worktree();
        let present = fingerprint(
            VerifierKind::FileExists,
            &Observed::FileExistence {
                exists: true,
                size: std::fs::metadata(d.path().join("app.yml")).expect("meta").len(),
            },
        )
        .expect("fingerprint");
        let f = fact(EvidenceKind::File, EvidenceCollector::Cairn, "app.yml", &present);
        let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::FileExists, None);
        assert_eq!(out.result, VerifyResult::Verified);

        std::fs::remove_file(d.path().join("app.yml")).expect("remove");
        let out = run_verifier(d.path(), &CairnConfig::default(), &f, VerifierKind::FileExists, None);
        assert_eq!(out.result, VerifyResult::Drifted, "a file that has gone has drifted");
    }

    #[test]
    fn an_observation_kind_has_no_verifier_of_its_own() {
        let f = fact(EvidenceKind::Observation, EvidenceCollector::Cairn, "x", "aaa");
        assert_eq!(verifier_for(&f), None);
    }
}
