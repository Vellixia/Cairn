//! Handoff synthesis (FR-033, FR-034, D7).
//!
//! Every field is *derived* from recorded state. An agent-supplied narrative is
//! accepted only as a bounded, clearly attributed `agent_note` — never as the
//! source of record, because a summary the agent writes about itself is exactly
//! the thing that drifts.

use crate::domain::*;
use chrono::Utc;

/// Everything handoff synthesis reads. All of it is recorded state.
pub struct HandoffInputs<'a> {
    pub session: &'a Session,
    pub task: Option<&'a Task>,
    /// Session observations, oldest first.
    pub observations: &'a [Observation],
    /// Decision-typed memories produced during this session.
    pub decision_memories: &'a [Memory],
    pub repository_state: RepositoryState,
    /// Paths Git currently reports as changed, for reconciliation.
    pub git_changed_files: &'a [String],
    pub agent_note: Option<String>,
}

/// Build a handoff for `trigger`.
pub fn synthesize(input: &HandoffInputs<'_>, trigger: HandoffTrigger) -> Handoff {
    let obs = input.observations;

    let changed_files =
        derive_changed_files(obs, input.git_changed_files, &input.session.worktree_path);
    let tests_executed = derive_tests(obs);
    let failures = derive_failures(obs);
    let decisions = derive_decisions(obs, input.decision_memories);
    let discoveries: Vec<String> = obs
        .iter()
        .filter(|o| o.kind == ObservationType::Discovery)
        .map(|o| o.summary.clone())
        .collect();

    let goal = derive_goal(input);
    let completed_work = derive_completed(&changed_files, &tests_executed, &discoveries);
    let remaining_work = derive_remaining(input, &failures, &tests_executed);
    let progress = derive_progress(&changed_files, &tests_executed, &failures);
    let next_step = derive_next_step(input, &failures, &remaining_work, &changed_files);

    Handoff {
        id: new_id(),
        session_id: input.session.id,
        trigger,
        goal,
        progress,
        completed_work,
        remaining_work,
        changed_files,
        decisions,
        failures,
        tests_executed,
        repository_state: input.repository_state.clone(),
        next_step,
        agent_note: input.agent_note.clone(),
        // Identifiers only. Never their content (FR-055).
        evidence: obs.iter().map(|o| o.id).collect(),
        created_at: Utc::now(),
        deleted_at: None,
    }
}

fn derive_goal(input: &HandoffInputs<'_>) -> String {
    if let Some(t) = input.task {
        return t.goal.clone();
    }
    // No task bound: the earliest user instruction is the best recorded proxy.
    if let Some(o) = input
        .observations
        .iter()
        .find(|o| o.kind == ObservationType::UserInstruction)
    {
        return o.summary.clone();
    }
    format!("Unbound session on branch {}", input.session.branch)
}

/// Observations carry absolute paths (FR-531); the wire, and the prose
/// `derive_completed` builds from this list, must carry neither.
///
/// Every path is relativized against `worktree_root` *unconditionally* —
/// not only when a separately-reported relative form of the same file
/// happens to exist. The earlier version only deduplicated a long/short pair
/// that were both already present, which left an absolute path standing
/// whenever Git reported nothing to pair it against — exactly the case of a
/// file with no Git counterpart (untracked, or outside what `git status`
/// reports at all).
fn derive_changed_files(
    obs: &[Observation],
    git_changed: &[String],
    worktree_root: &str,
) -> Vec<String> {
    let root = normalize_separators(worktree_root)
        .trim_end_matches('/')
        .to_string();
    let relativize = |path: String| -> String {
        let normalized = normalize_separators(&path);
        match normalized.strip_prefix(&root) {
            Some(rest) if rest.starts_with('/') => rest.trim_start_matches('/').to_string(),
            // Not under the recorded root: keep the normalized form anyway, so
            // the absolute-survivor pass below can recognise it and so the two
            // sides of every later comparison use one separator.
            _ => normalized,
        }
    };

    let mut files: Vec<String> = obs
        .iter()
        .filter(|o| o.kind == ObservationType::FileChanged)
        .filter_map(|o| o.path.clone())
        .map(relativize)
        .chain(git_changed.iter().map(|g| normalize_separators(g)))
        .collect();
    files.sort();
    files.dedup();

    // A path can still look absolute here: `worktree_path` and the path an
    // observation captured can each be built through a different symlink to
    // the same directory (`/tmp` is itself a symlink to `/private/tmp` on
    // macOS), so a prefix strip against the recorded root does not always
    // fire even though the file is the one Git already reported relatively.
    // Where a survivor is a component-aligned suffix of another entry, that
    // other entry is the same file's relative form; keep it and drop the
    // absolute one.
    let absolute_survivors: Vec<String> = files
        .iter()
        .filter(|p| looks_absolute(p))
        .filter(|long| {
            files
                .iter()
                .any(|short| short.len() < long.len() && long.ends_with(&format!("/{short}")))
        })
        .cloned()
        .collect();
    files.retain(|f| !absolute_survivors.contains(f));

    // **Whatever is still absolute here is reduced to its basename, because
    // FR-531 admits no absolute path in any transmitted field.**
    //
    // The pass above drops an absolute path only when a *relative* form of the
    // same file is also present — which is the symlink case it was written for.
    // A file with no Git counterpart at all, captured outside the recorded
    // worktree, has no such pair: an observation of `/outside/repo/notes.rs`
    // survived every check and travelled whole.
    //
    // The basename is kept rather than the entry dropped for the reason the
    // no-Git-counterpart test already states: this is a mechanism for relativizing
    // a path, not for silently eating a file. A reader learns that `notes.rs`
    // changed; nobody learns where the machine keeps it. Where a basename would
    // be empty or itself look rooted, the entry goes — there is nothing safe left
    // to say.
    files = files
        .into_iter()
        .filter_map(|f| {
            if !looks_absolute(&f) {
                return Some(f);
            }
            let base = f.rsplit('/').next().unwrap_or_default().to_string();
            (!base.is_empty() && !looks_absolute(&base)).then_some(base)
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

/// One separator, and no verbatim prefix, so two paths can be compared.
///
/// **This function is why FR-531 held on macOS and Linux and not on Windows.**
/// The relativizer compared with `'/'` and tested "looks absolute" with
/// `starts_with('/')`, so on Windows a captured drive path matched neither the
/// recorded root nor the absolute check: nothing relativized, nothing was even
/// recognised as absolute, and the full path reached `changed_files` and the
/// prose built from it. A leak the platform decided.
///
/// Forward slashes are also the right output form: Git reports paths that way on
/// every platform, so a relativized path and a Git-reported one are directly
/// comparable, which the dedup above depends on.
fn normalize_separators(path: &str) -> String {
    // The Windows verbatim prefix. Present on any path that has been through
    // `canonicalize`, absent on the one an agent's hook reports — so the two
    // would never share a prefix while only one carried it.
    let unc = "\\\\?\\UNC\\";
    let verbatim = "\\\\?\\";
    let stripped = if let Some(rest) = path.strip_prefix(unc) {
        format!("{}{}", "\\\\", rest)
    } else if let Some(rest) = path.strip_prefix(verbatim) {
        rest.to_string()
    } else {
        path.to_string()
    };
    stripped.replace(SEP, "/")
}

/// The native separator this function normalizes away.
const SEP: char = '\\';

/// Whether a path is absolute on **any** supported platform.
///
/// POSIX (`/etc/passwd`), a Windows drive (`C:/src`, after normalization), and a
/// UNC share (`//host/share`). Checked by shape rather than by
/// `Path::is_absolute`, which answers for the platform this build runs on — and
/// the path being screened may have been captured on another.
fn looks_absolute(path: &str) -> bool {
    if path.starts_with('/') {
        return true;
    }
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == SEP as u8)
}

fn derive_tests(obs: &[Observation]) -> Vec<TestRunRecord> {
    obs.iter()
        .filter(|o| o.kind == ObservationType::TestRun)
        .map(|o| TestRunRecord {
            // The runner's name, never the invocation. The field is `runner`
            // rather than `command` because the server's recursive field-name
            // denylist refuses a key called `command` wherever it appears —
            // sanitizing the value would not have been enough (FR-532).
            runner: o
                .command
                .as_deref()
                .map(test_runner_name)
                .unwrap_or_else(|| test_runner_name(&o.summary)),
            outcome: o.outcome.clone().unwrap_or_else(|| "unknown".to_string()),
            occurred_at: o.occurred_at,
        })
        .collect()
}

/// The invoked command with every argument, flag and path dropped (FR-532).
///
/// A test command line is exactly the kind of string this handoff must not
/// carry off the machine: an argument can be an absolute path, a flag value
/// can be a secret, and a leading `./script.sh` names a location on this
/// machine specifically. None of that is `derive_tests`'s job to redact
/// piecemeal — only to omit. What is safe to keep, and useful, is the
/// runner's own name: `cargo test --workspace -- --nocapture` becomes
/// `"cargo test"`; `pytest tests/test_foo.py -k slow` becomes `"pytest"`,
/// because the very next token names a path.
fn test_runner_name(command: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for token in command.split_whitespace() {
        let looks_like_an_argument = token.starts_with('-')
            || token.contains('/')
            || token.contains('\\')
            || token.contains('=');
        if looks_like_an_argument {
            break;
        }
        kept.push(token);
    }
    if let Some(name) = (!kept.is_empty()).then(|| kept.join(" ")) {
        return name;
    }
    // Even the first token looked like a path or a flag (e.g. `./run.sh`);
    // keep only its final path component so a local script's location does
    // not survive.
    command
        .split_whitespace()
        .next()
        .map(|first| {
            first
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(first)
                .to_string()
        })
        .unwrap_or_else(|| "test".to_string())
}

fn derive_failures(obs: &[Observation]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for o in obs {
        match o.kind {
            ObservationType::Error => out.push(o.summary.clone()),
            ObservationType::TestRun if o.outcome.as_deref() == Some("failed") => {
                let cmd = o.command.clone().unwrap_or_else(|| o.summary.clone());
                out.push(format!("Test failed: {cmd}"));
            }
            _ => {}
        }
    }
    out.dedup();
    out
}

fn derive_decisions(obs: &[Observation], memories: &[Memory]) -> Vec<String> {
    let mut out: Vec<String> = obs
        .iter()
        .filter(|o| o.kind == ObservationType::Decision)
        .map(|o| o.summary.clone())
        .collect();
    out.extend(
        memories
            .iter()
            .filter(|m| m.kind == MemoryType::Decision && m.deleted_at.is_none())
            .map(|m| m.content.clone()),
    );
    out.dedup();
    out
}

fn derive_completed(
    changed_files: &[String],
    tests: &[TestRunRecord],
    discoveries: &[String],
) -> Vec<String> {
    let mut out = Vec::new();
    if !changed_files.is_empty() {
        let shown: Vec<&str> = changed_files.iter().take(10).map(String::as_str).collect();
        let more = changed_files.len().saturating_sub(shown.len());
        let suffix = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        out.push(format!(
            "Changed {} file(s): {}{}",
            changed_files.len(),
            shown.join(", "),
            suffix
        ));
    }
    if !tests.is_empty() {
        let passed = tests.iter().filter(|t| t.outcome == "passed").count();
        let failed = tests.iter().filter(|t| t.outcome == "failed").count();
        out.push(format!(
            "Ran {} test command(s): {passed} passed, {failed} failed",
            tests.len()
        ));
    }
    out.extend(discoveries.iter().filter(|d| carries_meaning(d)).cloned());
    out
}

/// Whether a discovery says anything a reader could act on.
///
/// Tool calls without a command arrive summarised as the bare tool name —
/// `ToolSearch`, `mcp__cairn__cairn_remember` — and listing those as completed
/// work told the next session nothing except which buttons were pressed. A
/// summary earns its place by being prose: more than one word, and not just an
/// identifier.
fn carries_meaning(summary: &str) -> bool {
    let trimmed = summary.trim();
    if trimmed.split_whitespace().count() > 1 {
        return true;
    }
    // A single token is only informative if it is not a bare identifier.
    !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

fn derive_remaining(
    input: &HandoffInputs<'_>,
    failures: &[String],
    tests: &[TestRunRecord],
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = input.task {
        if t.status != TaskStatus::Done {
            // An acceptance criterion counts as satisfied only when a test
            // command passed and nothing failed. Anything less is remaining.
            let all_green = !tests.is_empty()
                && tests.iter().all(|r| r.outcome == "passed")
                && failures.is_empty();
            if !all_green {
                out.extend(
                    t.acceptance_criteria
                        .iter()
                        .map(|c| format!("Acceptance criterion: {c}")),
                );
            }
        }
    }
    // Same reasoning as `derive_next_step`: once the task is done, a failed
    // tool call is a recorded lesson, not outstanding work.
    let task_done = input
        .task
        .map(|t| t.status == TaskStatus::Done)
        .unwrap_or(false);
    if !task_done {
        out.extend(failures.iter().map(|f| format!("Open failure: {f}")));
    }
    if out.is_empty() && input.task.is_none() {
        out.push("No task bound; remaining work not tracked".to_string());
    }
    out
}

fn derive_progress(
    changed_files: &[String],
    tests: &[TestRunRecord],
    failures: &[String],
) -> String {
    let failed = failures.len();
    format!(
        "{} file(s) changed, {} test command(s) run, {} failure(s) open",
        changed_files.len(),
        tests.len(),
        failed
    )
}

fn derive_next_step(
    input: &HandoffInputs<'_>,
    failures: &[String],
    remaining: &[String],
    changed_files: &[String],
) -> String {
    // A tool that failed is not automatically work left to do. A session that
    // deliberately proves an approach does not work -- and records the decision
    // and the failure lesson saying so -- leaves a failed tool call behind on
    // purpose. Ordering the next session to "fix" it sends it to redo the dead
    // end the last one just ruled out, which is the opposite of carrying the
    // lesson forward. A completed task is the recorded signal that the failure
    // was accounted for rather than abandoned.
    let task_done = input
        .task
        .map(|t| t.status == TaskStatus::Done)
        .unwrap_or(false);
    if !task_done {
        if let Some(first) = failures.first() {
            return format!("Fix the open failure: {first}");
        }
    }
    if let Some(first) = remaining.first() {
        return format!("Continue with: {first}");
    }
    if !changed_files.is_empty() {
        return "Review the changed files and decide whether to commit".to_string();
    }
    "Pick up the task goal and start the first acceptance criterion".to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn bare_tool_names_are_not_completed_work() {
        let discoveries = vec![
            "ToolSearch".to_string(),
            "mcp__cairn__cairn_remember".to_string(),
            "Found the socket path is per-user".to_string(),
        ];
        let out = super::derive_completed(&[], &[], &discoveries);
        assert_eq!(out, vec!["Found the socket path is per-user".to_string()]);
    }

    use super::*;
    use chrono::Utc;

    fn session() -> Session {
        Session {
            id: new_id(),
            project_id: new_id(),
            task_id: None,
            user_id: new_id(),
            agent: "claude-code".into(),
            branch: "main".into(),
            commit_sha: Some("abc123".into()),
            worktree_path: "/tmp/repo".into(),
            agent_session_key: "sess-1".into(),
            previous_session_id: None,
            status: SessionStatus::Active,
            started_at: Utc::now(),
            ended_at: None,
            last_event_at: Utc::now(),
            last_turn_ended_at: None,
            daemon_run_id: new_id(),
            end_reason: None,
            deleted_at: None,
            handoff_pending: false,
            handoff_attempts: 0,
            handoff_error: None,
        }
    }

    fn obs(kind: ObservationType, summary: &str) -> Observation {
        Observation {
            id: new_id(),
            session_id: new_id(),
            kind,
            occurred_at: Utc::now(),
            branch: "main".into(),
            commit_sha: None,
            path: None,
            command: None,
            exit_code: None,
            outcome: None,
            summary: summary.into(),
            details: None,
            payload_bytes: summary.len() as i64,
            truncated: false,
            deleted_at: None,
        }
    }

    #[test]
    fn names_changed_file_failing_test_and_next_step() {
        let s = session();
        let mut changed = obs(ObservationType::FileChanged, "edited src/lib.rs");
        changed.path = Some("src/lib.rs".into());
        let mut test = obs(ObservationType::TestRun, "cargo test");
        test.command = Some("cargo test".into());
        test.outcome = Some("failed".into());

        let observations = vec![changed, test];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: None,
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: None,
            },
            HandoffTrigger::SessionEnd,
        );

        assert!(h.changed_files.contains(&"src/lib.rs".to_string()));
        assert_eq!(h.tests_executed.len(), 1);
        assert!(h.failures.iter().any(|f| f.contains("cargo test")));
        assert!(h.next_step.contains("cargo test"), "{}", h.next_step);
        assert_eq!(h.trigger, HandoffTrigger::SessionEnd);
    }

    #[test]
    fn reconciles_git_status_with_observations() {
        let s = session();
        let mut changed = obs(ObservationType::FileChanged, "edited a");
        changed.path = Some("a.rs".into());
        let observations = vec![changed];
        let git = vec!["b.rs".to_string(), "a.rs".to_string()];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: None,
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &git,
                agent_note: None,
            },
            // `Stop` is not a handoff trigger (FR-032); compaction is.
            HandoffTrigger::PreCompact,
        );
        assert_eq!(
            h.changed_files,
            vec!["a.rs".to_string(), "b.rs".to_string()]
        );
    }

    #[test]
    fn agent_note_is_attributed_and_cannot_replace_derived_fields() {
        let s = session();
        let observations: Vec<Observation> = vec![];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: None,
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: Some("I fixed everything".into()),
            },
            HandoffTrigger::SessionEnd,
        );
        assert_eq!(h.agent_note.as_deref(), Some("I fixed everything"));
        // The derived record disagrees with the narrative, and wins.
        assert!(h.changed_files.is_empty());
        assert!(h.progress.contains("0 file(s) changed"));
    }

    #[test]
    fn unmet_acceptance_criteria_become_remaining_work() {
        let s = session();
        let task = Task {
            id: new_id(),
            project_id: s.project_id,
            title: "Rate limiting".into(),
            goal: "Requests over the limit get 429".into(),
            acceptance_criteria: vec!["429 above threshold".into(), "Limit configurable".into()],
            status: TaskStatus::InProgress,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let observations: Vec<Observation> = vec![];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: Some(&task),
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: None,
            },
            HandoffTrigger::SessionEnd,
        );
        assert_eq!(h.goal, "Requests over the limit get 429");
        assert_eq!(h.remaining_work.len(), 2);
    }

    #[test]
    fn evidence_is_identifiers_only() {
        let s = session();
        let o = obs(ObservationType::FileRead, "read src/lib.rs");
        let id = o.id;
        let observations = vec![o];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: None,
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: None,
            },
            HandoffTrigger::Recovered,
        );
        assert_eq!(h.evidence, vec![id]);
    }

    /// A session that deliberately proves an approach wrong leaves a failed
    /// tool behind and marks the task done. Ordering the next session to fix it
    /// sends it back into the dead end the last one ruled out.
    #[test]
    fn a_done_task_does_not_order_the_next_session_to_fix_a_recorded_failure() {
        let s = session();
        let task = Task {
            id: new_id(),
            project_id: s.project_id,
            title: "Typed values".into(),
            goal: "parse_config coerces ints and bools".into(),
            acceptance_criteria: vec!["ints coerce".into()],
            status: TaskStatus::Done,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let observations = vec![obs(
            ObservationType::Error,
            "Bash failed: python3 -c \"import configparser\": tool execution failed",
        )];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: Some(&task),
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: None,
            },
            HandoffTrigger::SessionEnd,
        );
        assert!(
            !h.next_step.contains("Fix the open failure"),
            "next_step was {:?}",
            h.next_step
        );
        assert!(
            !h.remaining_work
                .iter()
                .any(|r| r.starts_with("Open failure")),
            "remaining_work was {:?}",
            h.remaining_work
        );
        // The failure itself is still on record, just not as outstanding work.
        assert_eq!(h.failures.len(), 1);
    }

    /// An unfinished task still owes the fix.
    #[test]
    fn an_unfinished_task_still_reports_the_open_failure() {
        let s = session();
        let task = Task {
            id: new_id(),
            project_id: s.project_id,
            title: "Typed values".into(),
            goal: "parse_config coerces ints and bools".into(),
            acceptance_criteria: vec![],
            status: TaskStatus::InProgress,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
        };
        let observations = vec![obs(ObservationType::Error, "Bash failed: cargo test")];
        let h = synthesize(
            &HandoffInputs {
                session: &s,
                task: Some(&task),
                observations: &observations,
                decision_memories: &[],
                repository_state: RepositoryState::default(),
                git_changed_files: &[],
                agent_note: None,
            },
            HandoffTrigger::SessionEnd,
        );
        assert!(h.next_step.starts_with("Fix the open failure"));
    }

    /// Observations carry absolute paths, Git reports the same file relative to
    /// the repository root, and both used to be counted.
    #[test]
    fn one_file_reported_two_ways_is_one_changed_file() {
        let mut o = obs(ObservationType::FileChanged, "edited confparse");
        // `/tmp` is itself a symlink to `/private/tmp` on macOS, so this does
        // not share a prefix with the session's recorded `/tmp/repo` root —
        // exercising the suffix-based fallback rather than the direct strip.
        o.path = Some("/private/tmp/repo/src/confparse.py".into());
        let observations = vec![o];
        let out = derive_changed_files(
            &observations,
            &["src/confparse.py".to_string()],
            "/tmp/repo",
        );
        assert_eq!(out, vec!["src/confparse.py".to_string()]);
    }

    /// The case the old suffix-only dedup could not collapse: a file with no
    /// Git counterpart at all, so there is no relative form to pair against.
    /// Direct relativization against the recorded worktree root must still
    /// fire (FR-531, T177).
    #[test]
    fn a_file_with_no_git_counterpart_is_still_relativized() {
        let mut o = obs(ObservationType::FileChanged, "added a new module");
        o.path = Some("/tmp/repo/src/new_module.rs".into());
        let observations = vec![o];
        let out = derive_changed_files(&observations, &[], "/tmp/repo");
        assert_eq!(out, vec!["src/new_module.rs".to_string()]);
    }

    /// A green suite run with Python's stdlib runner is a test command.
    #[test]
    fn a_stdlib_unittest_run_counts_as_a_test_command() {
        let mut o = obs(ObservationType::TestRun, "python3 -m unittest discover");
        o.command = Some("python3 -m unittest discover -s tests -q".into());
        o.outcome = Some("passed".into());
        let observations = vec![o];
        let progress = derive_progress(&[], &derive_tests(&observations), &[]);
        assert!(
            progress.contains("1 test command(s) run"),
            "progress was {progress:?}"
        );
    }
}

#[cfg(test)]
mod path_shape_tests {
    use super::*;

    /// Windows-shaped paths relativize, on every platform.
    ///
    /// **Asserted here rather than only end to end**, because the end-to-end test
    /// for FR-531 runs on whichever platform the runner is, and the bug it caught
    /// existed for exactly as long as no Windows runner had executed it. A pure
    /// function over strings can be given a Windows path on Linux, so the
    /// guarantee stops depending on where the suite happens to run.
    #[test]
    fn a_windows_path_relativizes_against_a_windows_root() {
        let root = r"C:\Users\dev\repo";
        let verbatim_root = r"\\?\C:\Users\dev\repo";
        for r in [root, verbatim_root] {
            assert_eq!(
                normalize_separators(r"\\?\C:\Users\dev\repo\src\lib.rs")
                    .strip_prefix(normalize_separators(r).trim_end_matches('/'))
                    .map(|s| s.trim_start_matches('/')),
                Some("src/lib.rs"),
                "a captured Windows path did not relativize against root {r}"
            );
        }
    }

    /// A POSIX path still relativizes exactly as it did.
    #[test]
    fn a_posix_path_is_unaffected() {
        assert_eq!(
            normalize_separators("/home/dev/repo/src/lib.rs"),
            "/home/dev/repo/src/lib.rs"
        );
        assert_eq!(normalize_separators("src/lib.rs"), "src/lib.rs");
    }

    /// Every absolute shape is recognised, and a relative one is not.
    ///
    /// The Windows rows are the ones that mattered: `looks_absolute` used to be
    /// `starts_with('/')`, so a drive path was classified relative and the
    /// absolute-survivor pass never considered it.
    #[test]
    fn every_absolute_shape_is_recognised_on_every_platform() {
        for absolute in [
            "/etc/passwd",
            "/home/dev/repo/src/lib.rs",
            "C:/Users/dev/repo/src/lib.rs",
            r"C:\Users\dev\repo\src\lib.rs",
            "d:/tmp/x",
        ] {
            assert!(
                looks_absolute(absolute),
                "`{absolute}` was not recognised as absolute, so FR-531's screen \
                 would not consider it"
            );
        }
        for relative in ["src/lib.rs", r"src\lib.rs", ".gitignore", "", "C:", "CC:/x"] {
            assert!(
                !looks_absolute(relative),
                "`{relative}` was treated as absolute"
            );
        }
    }

    /// The verbatim prefix is stripped, including its UNC form.
    #[test]
    fn the_windows_verbatim_prefix_is_stripped() {
        assert_eq!(
            normalize_separators(r"\\?\C:\Users\dev\repo"),
            "C:/Users/dev/repo"
        );
        assert_eq!(
            normalize_separators(r"\\?\UNC\server\share\repo"),
            "//server/share/repo"
        );
    }

    /// A capture from **outside** the recorded worktree, with no Git counterpart,
    /// never travels whole.
    ///
    /// The suffix-dedup pass drops an absolute path only when a relative form of
    /// the same file is also present — the symlink case it was written for. A file
    /// captured outside the worktree has no such pair, so it survived every check
    /// and the full local path reached `changed_files` and the prose built from
    /// it, against FR-531's "MUST NOT transmit an absolute local path in any
    /// field".
    ///
    /// The basename is kept rather than the entry dropped: this is a mechanism for
    /// relativizing a path, not for silently eating a file.
    ///
    /// Falsified by removing the final redaction pass.
    #[test]
    fn a_capture_outside_the_worktree_is_reduced_to_its_basename() {
        let outside = "/outside/the/repo/notes.rs";
        let files = derive_changed_files(
            &[fixture_at(outside)],
            // No Git counterpart at all: nothing to pair the absolute path with.
            &[],
            "/home/dev/repo",
        );
        assert_eq!(
            files,
            vec!["notes.rs".to_string()],
            "an absolute path from outside the worktree survived: {files:?}"
        );
        assert!(
            !files.iter().any(|f| looks_absolute(f)),
            "an absolute path reached changed_files (FR-531): {files:?}"
        );

        // The same for a Windows capture outside the root, and for a root-level
        // file whose basename is all there is to say.
        for (path, expected) in [
            (r"D:\elsewhere\notes.rs", "notes.rs"),
            ("/etc/hosts", "hosts"),
        ] {
            let files = derive_changed_files(&[fixture_at(path)], &[], "/home/dev/repo");
            assert_eq!(files, vec![expected.to_string()], "for {path}");
        }
    }

    /// An observation fixture whose only interesting field is its path.
    fn fixture_at(path: &str) -> Observation {
        Observation {
            id: crate::domain::new_id(),
            session_id: crate::domain::new_id(),
            kind: ObservationType::FileChanged,
            occurred_at: chrono::Utc::now(),
            branch: "main".into(),
            commit_sha: None,
            path: Some(path.to_string()),
            command: None,
            exit_code: None,
            outcome: None,
            summary: "edited a file".into(),
            details: None,
            payload_bytes: 0,
            truncated: false,
            deleted_at: None,
        }
    }

    /// End to end through `derive_changed_files`: a Windows absolute path under
    /// the recorded root comes out relative, and never absolute.
    #[test]
    fn derive_changed_files_relativizes_a_windows_capture() {
        let obs = vec![Observation {
            id: crate::domain::new_id(),
            session_id: crate::domain::new_id(),
            kind: ObservationType::FileChanged,
            occurred_at: chrono::Utc::now(),
            branch: "main".into(),
            commit_sha: None,
            path: Some(r"\\?\C:\Users\runneradmin\Temp\repo\generated\ignored_module.rs".into()),
            command: None,
            exit_code: None,
            outcome: None,
            summary: "edited a generated module".into(),
            details: None,
            payload_bytes: 0,
            truncated: false,
            deleted_at: None,
        }];
        let files = derive_changed_files(
            &obs,
            &[".gitignore".into()],
            r"C:\Users\runneradmin\Temp\repo",
        );
        assert!(
            files.contains(&"generated/ignored_module.rs".to_string()),
            "the Windows capture did not relativize: {files:?}"
        );
        assert!(
            !files.iter().any(|f| looks_absolute(f)),
            "an absolute path survived into changed_files (FR-531): {files:?}"
        );
    }
}
