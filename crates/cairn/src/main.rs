//! `cairn` — the developer's interface, the hook runtime, and the MCP server.

// The MCP tool definitions are one `json!` literal per tool, and
// `cairn_remember` now carries every Feature 003 action's parameters. The
// macro expands recursively once per node, so the default limit is reached
// by a schema that is merely long rather than deep.
#![recursion_limit = "512"]

mod client;
mod hook;
mod integrate;
mod mcp;
mod render;
mod update;

use cairn_core::domain::*;
use cairn_core::wire::*;
use clap::{Parser, Subcommand};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "cairn",
    version,
    about = "Persistent, project-aware memory for AI coding agents"
)]
struct Cli {
    /// Emit the stable JSON envelope instead of human output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Register this repository as a Cairn project.
    Init,
    /// Project, repository, sessions and daemon state.
    Status,
    /// Detected agents and integration managers, with their level.
    Agents,
    /// Install or update an integration for this repository.
    Connect {
        /// `claude-code` | `codex` | `opencode` | `generic-mcp`. Omit for
        /// guided onboarding across everything detected.
        agent: Option<String>,
        /// Detect everything installed and propose a plan covering all of it.
        #[arg(long)]
        auto: bool,
        /// Print the change plan and exit. Writes nothing at all.
        #[arg(long)]
        dry_run: bool,
        /// Apply without confirmation. Required for non-interactive use.
        #[arg(long)]
        yes: bool,
        /// Install lifecycle and MCP into committed project scope.
        #[arg(long)]
        shared: bool,
        /// Override one resource's scope, e.g. `--scope mcp=project_shared`.
        #[arg(long = "scope")]
        scopes: Vec<String>,
        /// Distribute `mcp` and `skill` through a manager instead.
        #[arg(long)]
        via: Option<String>,
        /// Manager target applications.
        #[arg(long, value_delimiter = ',')]
        apps: Vec<String>,
    },
    /// Inspect integration health. Makes no change.
    Doctor {
        agent: Option<String>,
        /// Recompute every derived value and report how many differed.
        ///
        /// Exits non-zero if any did: a release where a derived value
        /// disagrees with its rebuild ships a known inconsistency.
        ///
        /// Scoped to the project this directory resolves to, like every other
        /// command. There is deliberately no `--project`: it would be a second
        /// way to say what `cd` already says, and a flag with two meanings is
        /// how the wrong project gets rebuilt.
        #[arg(long)]
        rebuild_derived: bool,
    },
    /// Reusable cross-project patterns (`contracts/patterns.md`).
    ///
    /// A pattern is local to this machine and never synchronizes.
    Pattern {
        #[command(subcommand)]
        action: PatternAction,
    },
    /// Restore Cairn-owned state only.
    Repair {
        agent: Option<String>,
        #[arg(long)]
        dry_run: bool,
        /// Also restore resources edited by hand, strictly inside Cairn's own
        /// ownership boundary and after preserving the previous content.
        #[arg(long)]
        force: bool,
    },
    /// Remove Cairn-owned integration for one agent.
    Disconnect {
        #[arg(default_value = "claude-code")]
        agent: String,
        /// Restrict removal to these resource kinds. Repeatable.
        #[arg(long = "only")]
        only: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
    /// Operations a developer runs rarely.
    Integration {
        #[command(subcommand)]
        action: IntegrationAction,
    },
    /// Sessions.
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Tasks.
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    /// Durable memory.
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Bounded, redacted evidence facts. Local, always.
    Evidence {
        #[command(subcommand)]
        action: EvidenceAction,
    },
    /// Run deterministic verification.
    ///
    /// Cairn reads files inside the worktree and Git. It runs no build, no test
    /// suite and no shell command, and it reaches no network.
    Verify {
        /// One memory.
        #[arg(long)]
        memory: Option<Uuid>,
        /// Every memory in the project that owes a check, within the pass caps.
        #[arg(long)]
        all: bool,
        /// Print the run history, not only the current state.
        #[arg(long)]
        explain: bool,
    },
    /// Read a handoff.
    Handoff {
        #[command(subcommand)]
        action: HandoffAction,
    },
    /// Print the briefing a session would receive.
    Context {
        #[arg(long, alias = "token-budget")]
        budget: Option<usize>,
        /// Which session to brief, when more than one is open here.
        #[arg(long)]
        session: Option<Uuid>,
        /// Why the briefing is being assembled. `post_compaction` restores the
        /// session's checkpoint.
        #[arg(long)]
        reason: Option<String>,
        /// Show why each item was selected and why each omission was left out.
        ///
        /// Costs no budget when absent: the diagnostics are computed but only
        /// returned on request (FR-462, FR-463).
        #[arg(long)]
        explain: bool,
        /// `minimum` excludes personal notes and team guidance entirely
        /// (FR-477). Absent means `standard`, today's full assembly.
        #[arg(long)]
        depth: Option<String>,
    },
    /// Capture exclusions.
    Privacy {
        #[command(subcommand)]
        action: PrivacyAction,
    },
    /// Delete stored data.
    Delete {
        #[command(subcommand)]
        action: DeleteAction,
    },
    /// Opt this project into server sync.
    Link {
        /// Join an existing shared project by identifier.
        #[arg(long)]
        project: Option<Uuid>,
        /// Create a new shared project.
        #[arg(long)]
        create: bool,
    },
    /// Opt this project back out.
    Unlink,
    /// Server credentials.
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Server-side accounts: admin-only (`contracts/identity-
    /// administration.md` §9).
    ///
    /// Every subcommand forwards to `/api/admin/users` and reports whatever
    /// the server decides — there is no local authorization decision here,
    /// only whatever a `403 forbidden` from a non-admin caller looks like.
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// Server-wide team knowledge (`contracts/global-memory.md` §5b).
    ///
    /// `list`/`propose` are reachable by any member; `ratify`/`retire` are
    /// admin-only, authorized by the server alone.
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
    /// This account's own personal, project-independent knowledge.
    Personal {
        #[command(subcommand)]
        action: PersonalAction,
    },
    /// This project's derived stack traits — what applicability matching
    /// reads at recall time.
    Traits,
    /// Shared projects: membership.
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Synchronization.
    Sync {
        #[command(subcommand)]
        action: SyncAction,
    },
    /// Daemon lifecycle.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Check for a newer release, and install it.
    Update {
        /// Report what is available without installing anything.
        #[arg(long)]
        check: bool,
    },
    /// Run the MCP server over stdio.
    Mcp,
    /// Claude Code hook entry point. Always exits 0.
    /// A vendor lifecycle event, translated by the named agent's adapter.
    ///
    /// Always exits 0, whatever happened: Cairn is never the reason a session
    /// breaks (FR-015, FR-193).
    Hook {
        event: String,
        /// Which adapter to translate with. Defaults to Claude Code so a
        /// Feature 001 hook entry keeps working unchanged.
        #[arg(long)]
        agent: Option<String>,
    },
}

/// The operations a developer runs rarely (`contracts/integration-cli.md`).
#[derive(Subcommand)]
enum IntegrationAction {
    /// Emit a deterministic, secret-free MCP configuration. Writes nothing.
    Export {
        #[command(subcommand)]
        what: ExportAction,
    },
    /// Move one resource between owners.
    Migrate {
        agent: String,
        kind: String,
        #[arg(long = "to")]
        to: String,
        #[arg(long)]
        dry_run: bool,
        /// Continue an interrupted migration.
        #[arg(long)]
        resume: bool,
        /// Reverse one, leaving the previously working configuration intact.
        #[arg(long)]
        abort: bool,
    },
    /// Distribute Cairn resources through an integration manager.
    Distribute {
        #[arg(long)]
        via: String,
        #[arg(long)]
        resource: String,
        #[arg(long, value_delimiter = ',')]
        apps: Vec<String>,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum ExportAction {
    /// The Cairn MCP server block, in the named agent's format.
    Mcp {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        format: Option<String>,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// Every session in this project, newest first.
    List,
    /// Record a continuity checkpoint now.
    ///
    /// Derives the boundary record first when none exists, rather than
    /// refusing: asking for a checkpoint is reasonable at any point, and
    /// producing the handoff it anchors to is Cairn's job (FR-425).
    Checkpoint {
        #[arg(long)]
        session: Option<Uuid>,
    },
    Start {
        #[arg(long, default_value = "cairn-cli")]
        agent: String,
        #[arg(long)]
        key: Option<String>,
        #[arg(long)]
        task: Option<Uuid>,
    },
    Show {
        #[arg(long)]
        session: Option<Uuid>,
    },
    End {
        #[arg(long)]
        session: Option<Uuid>,
        #[arg(long, default_value = "completed")]
        status: String,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Subcommand)]
enum TaskAction {
    List {
        #[arg(long)]
        status: Option<String>,
    },
    Show {
        id: Uuid,
    },
    New {
        #[arg(long)]
        title: String,
        #[arg(long)]
        goal: String,
        /// Repeatable.
        #[arg(long = "criterion")]
        criteria: Vec<String>,
    },
    SetStatus {
        id: Uuid,
        status: String,
    },
    /// Feature 001's whole-list form. Still works: the list is diffed by text,
    /// so unchanged entries keep their ids and labels.
    Update {
        id: Uuid,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        goal: Option<String>,
        /// Repeatable. The whole list, as Feature 001 has always taken it.
        #[arg(long = "acceptance-criteria")]
        acceptance_criteria: Option<Vec<String>>,
        #[arg(long)]
        status: Option<String>,
    },
    /// Acceptance criteria with stable identity.
    #[command(subcommand)]
    Criterion(CriterionAction),
    /// Blockers — append-only, one `open → cleared` transition.
    #[command(subcommand)]
    Blocker(BlockerAction),
    /// Derived progress and completion readiness. Changes no status.
    Readiness {
        id: Uuid,
    },
    /// The local change log, including blind-write markers.
    History {
        id: Uuid,
        #[arg(long)]
        limit: Option<i64>,
    },
}

#[derive(Subcommand)]
enum CriterionAction {
    Add {
        task_id: Uuid,
        #[arg(long)]
        text: String,
        #[arg(long)]
        session: Option<Uuid>,
    },
    Set {
        criterion_id: Uuid,
        /// `pending`, `satisfied`, `blocked` or `waived`. Independent of
        /// verification, which only Cairn writes.
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        text: Option<String>,
        /// The revision you read. Supplying it is how you are protected from
        /// losing someone else's assertion; omitting it applies the write and
        /// records it as a blind write.
        #[arg(long = "expected-revision")]
        expected_revision: Option<i64>,
        #[arg(long)]
        session: Option<Uuid>,
    },
    /// Ask Cairn to verify the criterion from its evidence.
    ///
    /// There is no flag that asserts a verification: a criterion reaches
    /// `verified` only on a deterministic check this machine ran over evidence
    /// Cairn collected itself.
    Verify {
        criterion_id: Uuid,
        #[arg(long)]
        evidence: Option<Uuid>,
        #[arg(long)]
        session: Option<Uuid>,
    },
    /// Tombstone it. Ordinals are not renumbered, so no label changes meaning.
    Remove {
        criterion_id: Uuid,
        #[arg(long)]
        session: Option<Uuid>,
    },
}

#[derive(Subcommand)]
enum BlockerAction {
    Open {
        task_id: Uuid,
        #[arg(long)]
        description: String,
        #[arg(long)]
        session: Option<Uuid>,
    },
    /// The only transition, and terminal: reopening creates a new blocker.
    Clear {
        blocker_id: Uuid,
        #[arg(long)]
        session: Option<Uuid>,
    },
}

#[derive(Subcommand)]
enum PatternAction {
    /// List promoted patterns with their counters.
    List {
        /// `candidate`, `sanitized`, `validated` or `contested`.
        #[arg(long)]
        trust: Option<String>,
        /// Only patterns matching this signal token.
        #[arg(long)]
        signal: Option<String>,
    },
    /// Full text, applications, counterexamples and the sanitization report.
    Show { id: Uuid },
    /// Propose promoting a project memory to a reusable pattern.
    ///
    /// Runs the ten-check gate. Any failure refuses, names the class and writes
    /// nothing.
    Promote {
        #[arg(long)]
        memory: Uuid,
        /// A symptom token or error signature. Repeatable; at least two must
        /// survive normalization.
        #[arg(long = "signal")]
        signals: Vec<String>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        problem: Option<String>,
        /// A condition under which the pattern applies. Repeatable.
        #[arg(long = "applies-when")]
        applicability: Vec<String>,
        #[arg(long)]
        root_cause: Option<String>,
        #[arg(long)]
        approach: Option<String>,
        /// What the approach does *not* do. Repeatable.
        #[arg(long = "caveat")]
        constraints: Vec<String>,
        /// Report the gate outcome without writing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Record what happened when a pattern was applied here.
    Outcome {
        id: Uuid,
        /// `resolved`, `not_applicable` or `failed`.
        #[arg(long)]
        outcome: String,
        /// The signals this project saw. Repeatable — one incident, one set.
        #[arg(long = "signal")]
        signals: Vec<String>,
        /// The cause found instead, on a `not_applicable` outcome.
        #[arg(long)]
        alternative_cause: Option<String>,
        /// Deterministic evidence collected in **this** project.
        #[arg(long)]
        evidence: Option<Uuid>,
        #[arg(long)]
        session: Option<Uuid>,
    },
    /// Tombstone a pattern. Its applications survive as history.
    Forget { id: Uuid },
}

#[derive(Subcommand)]
enum EvidenceAction {
    /// Record a fact, optionally attaching it to a memory.
    Add {
        /// `observation`, `file`, `git_ref`, `configuration`, `test_outcome`,
        /// `command_outcome`, `runtime_state`, `schema_version`.
        #[arg(long = "type")]
        kind: String,
        /// What it describes — "database backend".
        #[arg(long)]
        subject: String,
        /// The observed value. Redacted, then bounded.
        #[arg(long)]
        value: String,
        /// Repository-relative path or Git ref. Never absolute. A configuration
        /// locator names its key after a `#`: `config/app.yml#server.port`.
        #[arg(long)]
        locator: String,
        /// `cairn` when Cairn can read it; `agent` when an agent attests it.
        #[arg(long)]
        collector: Option<String>,
        #[arg(long)]
        observation: Option<Uuid>,
        /// The memory it bears on.
        #[arg(long)]
        memory: Option<Uuid>,
        /// `supports` (default) or `contradicts`.
        #[arg(long)]
        role: Option<String>,
        #[arg(long)]
        session: Option<Uuid>,
    },
    List {
        /// Only the facts attached to this memory.
        #[arg(long)]
        memory: Option<Uuid>,
    },
    Show {
        id: Uuid,
    },
}

#[derive(Subcommand)]
enum MemoryAction {
    /// Hold a memory in Level 0 as a standing constraint.
    ///
    /// A pin never widens scope, and nothing is ever auto-unpinned to make room
    /// (FR-453, FR-454).
    Pin {
        id: Uuid,
        /// Unpin it instead.
        #[arg(long = "off")]
        off: bool,
        /// Bounded and redacted before it is stored.
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        session: Option<Uuid>,
    },
    Add {
        content: String,
        #[arg(long = "type", default_value = "fact")]
        kind: String,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        scope_key: Option<String>,
        /// Never transmitted, even for a linked project.
        #[arg(long)]
        local_only: bool,
        /// Supporting observation ids. Optional, and never invented.
        #[arg(long = "evidence")]
        evidence: Vec<Uuid>,
        /// The subject this states something about — `infra.production_database`.
        ///
        /// Optional. Without one the memory is free-form and behaves exactly as
        /// it does today; with one it takes part in reconciliation.
        #[arg(long)]
        topic_key: Option<String>,
        /// The comparable value it asserts. Only meaningful with a topic key.
        #[arg(long)]
        value_key: Option<String>,
        /// Ranks within a bucket, and nothing more: it never changes scope
        /// precedence and never admits an item into reserved context.
        #[arg(long)]
        importance: Option<String>,
        /// Which session recorded this, when more than one is open here.
        #[arg(long)]
        session: Option<Uuid>,
    },
    /// Replace a memory, keeping the original and the link between them.
    ///
    /// The original is retained and marked superseded, and a `supersedes`
    /// relation records *who* decided and *when* — which is what lets an
    /// `--as-of` search still answer what the project believed in July
    /// (FR-020, FR-323, FR-342).
    Supersede {
        content: String,
        /// The memory this replaces.
        #[arg(long = "memory-id")]
        memory_id: Uuid,
        #[arg(long = "type", default_value = "fact")]
        kind: String,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        scope_key: Option<String>,
        /// Never transmitted, even for a linked project.
        #[arg(long)]
        local_only: bool,
        /// Supporting observation ids. Optional, and never invented.
        #[arg(long = "evidence")]
        evidence: Vec<Uuid>,
        /// The subject the replacement states something about.
        #[arg(long)]
        topic_key: Option<String>,
        /// The comparable value it asserts. Only meaningful with a topic key.
        #[arg(long)]
        value_key: Option<String>,
        #[arg(long)]
        importance: Option<String>,
        /// Which session recorded this, when more than one is open here.
        #[arg(long)]
        session: Option<Uuid>,
    },
    /// Inspect a subject: its members, its answer or answers, and why.
    Subject {
        topic_key: String,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        scope_key: Option<String>,
        /// `project` (the default), `personal` or `team`.
        ///
        /// A domain is not a scope: `--scope` describes how long a project
        /// memory stays relevant and means nothing to the other two, which have
        /// none. Naming a domain reads that corpus and its own relations.
        #[arg(long)]
        domain: Option<String>,
    },
    /// Confirm that an existing memory is still true.
    ///
    /// Explicit, always. Cairn never infers a reinforcement from a matching
    /// value key.
    Reinforce {
        id: Uuid,
        /// The memory carrying this session's confirming statement.
        #[arg(long)]
        from: Uuid,
        #[arg(long)]
        session: Option<Uuid>,
    },
    /// Record an explicit reconciliation decision.
    Reconcile {
        #[arg(long)]
        from: Uuid,
        #[arg(long)]
        to: Uuid,
        /// `supersedes`, `narrows`, `not_applicable_to`, `duplicates` or
        /// `reinforces`. A conflict is detected, never declared.
        #[arg(long)]
        relation: String,
        /// `deterministic_rule`, `evidence`, `explicit_agent` or
        /// `explicit_user`.
        #[arg(long, default_value = "explicit_user")]
        basis: String,
        #[arg(long)]
        basis_evidence: Option<Uuid>,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        session: Option<Uuid>,
    },
    Search {
        query: Option<String>,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        scope_key: Option<String>,
        #[arg(long = "type")]
        kind: Option<String>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        limit: Option<i64>,
        /// Exact subject identity, or a prefix when it ends in a dot.
        #[arg(long)]
        topic_key: Option<String>,
        /// What was effective at an instant, RFC 3339. A historical answer,
        /// echoed back so it cannot be mistaken for a current one.
        #[arg(long)]
        as_of: Option<String>,
        /// Only memories whose subject is conflicted.
        #[arg(long)]
        conflicted: bool,
        /// Only memories whose subject is corroborated.
        #[arg(long)]
        corroborated: bool,
        /// `unverified`, `verified`, `needs_recheck`, `drifted`, `conflicted`.
        #[arg(long)]
        verification: Option<String>,
        /// Also return signal-matched prior patterns, in a separate array.
        #[arg(long)]
        include_patterns: bool,
        /// What established it: `cairn`, `attested`, `remote_cairn`,
        /// `remote_attested`.
        #[arg(long)]
        authority: Option<String>,
        /// Which session's task to rank by, when more than one is open here.
        #[arg(long)]
        session: Option<Uuid>,
    },
    Show {
        id: Uuid,
    },
    Forget {
        id: Uuid,
    },
}

#[derive(Subcommand)]
enum HandoffAction {
    Show {
        #[arg(long)]
        session: Option<Uuid>,
    },
}

#[derive(Subcommand)]
enum PrivacyAction {
    Exclude {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        command: Option<String>,
    },
    Unexclude {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        command: Option<String>,
    },
    List,
}

#[derive(Subcommand)]
enum DeleteAction {
    Observation {
        id: Uuid,
    },
    Memory {
        id: Uuid,
    },
    Handoff {
        id: Uuid,
    },
    Session {
        id: Uuid,
        /// Also delete the memories this session produced. Never the default.
        #[arg(long)]
        with_memories: bool,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// Store the personal API token generated in the web UI.
    Token {
        #[command(subcommand)]
        action: TokenAction,
    },
    Logout,
    /// Whether a credential is stored, and for which server.
    Status,
    /// Change the caller's own password (FR-405). The only route reachable
    /// while an admin-created account still owes its first change.
    ChangePassword {
        /// Read from stdin when omitted, so it never lands in shell history —
        /// the same convention `auth token set` already uses.
        #[arg(long = "new-password")]
        new_password: Option<String>,
    },
}

/// `cairn user` (`contracts/identity-administration.md` §9). Every command
/// takes an email — never a server-side row id, which the CLI never learns —
/// except `create`, which mints the account the email will belong to, and
/// `list`, which names none.
#[derive(Subcommand)]
enum UserAction {
    /// Create an account. The temporary password this prints is shown
    /// exactly once: there is no route, ever, that reads it back (FR-403).
    /// If it is lost, the remedy is `cairn user reset-password`.
    Create {
        #[arg(long)]
        email: String,
        #[arg(long = "display-name")]
        display_name: String,
    },
    /// Every account, its role and its status (FR-411).
    List,
    /// Revoke an account's ability to authenticate. Every live API token it
    /// holds is revoked in the same server transaction (FR-409, FR-410).
    Disable { email: String },
    /// Restore an account's ability to authenticate. Tokens revoked by a
    /// prior disable are not restored (FR-590).
    Enable { email: String },
    /// Grant server-level admin standing (FR-412).
    Promote { email: String },
    /// Remove server-level admin standing. Refused when this account is the
    /// server's only remaining active administrator (FR-413).
    Demote { email: String },
    /// Issue a new temporary password and revoke every token the account
    /// holds (FR-553–FR-559). The password is shown exactly once, the same
    /// discipline `create` follows.
    ResetPassword { email: String },
}

/// `cairn team` (`contracts/global-memory.md` §5b, T133).
#[derive(Subcommand)]
enum TeamAction {
    /// `authoritative` entries, plus your own `proposed` ones.
    List {
        /// Every state, from every proposer. Admin only (FR-464); a
        /// non-admin caller is told so rather than silently downgraded.
        #[arg(long)]
        all: bool,
    },
    /// Propose an entry. Lands `proposed`; only an admin can ratify it.
    Propose {
        content: String,
        #[arg(long = "type", default_value = "fact")]
        kind: String,
        #[arg(long = "topic-key")]
        topic_key: Option<String>,
        #[arg(long = "value-key")]
        value_key: Option<String>,
        /// `kind=value`, `language` or `tool` only. Repeatable.
        #[arg(long = "applies-to")]
        applies_to: Vec<String>,
    },
    /// Admin only, authorized by the server. `proposed` → `authoritative`.
    Ratify {
        id: Uuid,
        /// Record that this ratified entry supersedes an existing one
        /// (an explicit admin decision, never inferred).
        #[arg(long)]
        supersedes: Option<Uuid>,
    },
    /// Admin only, authorized by the server. `authoritative` → `retired`;
    /// never reversible by re-ratifying.
    Retire { id: Uuid },
}

/// `cairn personal` — this account's own personal, project-independent
/// knowledge (`contracts/global-memory.md` §5a, T082).
#[derive(Subcommand)]
enum PersonalAction {
    List {
        #[arg(long)]
        query: Option<String>,
        #[arg(long)]
        limit: Option<i64>,
    },
    /// Tombstone one entry: content cleared, nothing else touched.
    Forget { id: Uuid },
}

/// `cairn project member` (`contracts/identity-administration.md` §9a,
/// T063).
#[derive(Subcommand)]
enum ProjectAction {
    /// Shared-project membership.
    Member {
        #[command(subcommand)]
        action: MemberAction,
    },
}

#[derive(Subcommand)]
enum MemberAction {
    /// Grant membership. Reachable by an existing member or a server admin.
    Add { project_id: Uuid, email: String },
    /// Revoke membership.
    Remove { project_id: Uuid, email: String },
    /// The full membership list.
    List { project_id: Uuid },
}

#[derive(Subcommand)]
enum TokenAction {
    Set {
        /// Read from stdin when omitted, so it never lands in shell history.
        token: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
}

#[derive(Subcommand)]
enum SyncAction {
    Status,
    Now,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Show the daemon's recent log.
    Logs {
        /// How many lines from the end.
        #[arg(long, default_value_t = 50)]
        tail: usize,
    },
    Start,
    Stop,
    Status,
}

/// Exit codes: 0 success, 1 user error, 2 Cairn unavailable.
const EXIT_USER_ERROR: i32 = 1;
const EXIT_UNAVAILABLE: i32 = 2;

fn main() {
    // The capture class is handled before any async runtime is built: a hook
    // runs once per tool call, and a runtime per call is the largest cost
    // Cairn adds to a session (SC-007).
    let argv: Vec<String> = std::env::args().collect();
    if argv.len() >= 3 && argv[1] == "hook" && hook::run_blocking(&argv[2]) {
        std::process::exit(0);
    }
    run_async()
}

#[tokio::main]
async fn run_async() {
    let cli = Cli::parse();

    // The hook entry point is the exception to every rule below: it always
    // exits 0, whatever happened (FR-015).
    if let Command::Hook { event, .. } = &cli.command {
        hook::run(event).await;
        std::process::exit(0);
    }
    if let Command::Mcp = &cli.command {
        if let Err(e) = mcp::serve().await {
            eprintln!("cairn mcp: {e}");
            std::process::exit(EXIT_UNAVAILABLE);
        }
        return;
    }

    match run(&cli).await {
        Ok(output) => {
            let actionable = output.exit_nonzero;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&Envelope::ok(output.value)).unwrap()
                );
            } else if !output.text.is_empty() {
                print!("{}", output.text);
            }
            // Doctor succeeds as a command and still exits 1 when it found
            // something actionable, so a script can gate on it (FR-170).
            if actionable {
                std::process::exit(EXIT_USER_ERROR);
            }
        }
        Err(e) => {
            let code = exit_code(&e);
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&Envelope::err(e)).unwrap()
                );
            } else {
                eprintln!("cairn: {}: {}", e.code, e.message);
            }
            std::process::exit(code);
        }
    }
}

pub(crate) fn exit_code(e: &WireError) -> i32 {
    match e.code.as_str() {
        codes::DAEMON_UNAVAILABLE | codes::STORAGE_UNAVAILABLE | codes::SERVER_UNAVAILABLE => {
            EXIT_UNAVAILABLE
        }
        _ => EXIT_USER_ERROR,
    }
}

pub(crate) struct Output {
    pub value: serde_json::Value,
    pub text: String,
    /// Doctor exits 1 when any actionable condition is present, while still
    /// succeeding as a command (FR-170).
    pub exit_nonzero: bool,
}

impl Output {
    fn plain(value: serde_json::Value) -> Self {
        Self {
            value,
            text: String::new(),
            exit_nonzero: false,
        }
    }
    pub(crate) fn with(value: serde_json::Value, text: String) -> Self {
        Self {
            value,
            text,
            exit_nonzero: false,
        }
    }
}

fn cwd() -> String {
    std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".into())
}

fn parse_enum<T: std::str::FromStr>(what: &str, raw: &str) -> Result<T, WireError>
where
    T::Err: std::fmt::Display,
{
    raw.parse::<T>()
        .map_err(|e| WireError::invalid(format!("bad {what}: {e}")))
}

async fn run(cli: &Cli) -> Result<Output, WireError> {
    match &cli.command {
        Command::Hook { .. } | Command::Mcp => unreachable!("handled in main"),

        Command::Init => {
            let v = client::send(&Request::Init { cwd: cwd() }).await?;
            let name = v["project"]["name"].as_str().unwrap_or("project");
            Ok(Output::with(
                v.clone(),
                format!("Cairn is tracking {name}.\n"),
            ))
        }

        Command::Status => {
            let v = client::send(&Request::Status { cwd: cwd() }).await?;
            let payload: StatusPayload =
                serde_json::from_value(v.clone()).map_err(|e| WireError::invalid(e.to_string()))?;
            Ok(Output::with(v, render::status(&payload)))
        }

        Command::Agents => integrate::agents().await,
        Command::Connect {
            agent,
            auto,
            dry_run,
            yes,
            shared,
            scopes,
            via,
            apps,
        } => {
            let opts = integrate::Options {
                agent: parse_agent_opt(agent)?,
                auto: *auto,
                dry_run: *dry_run,
                yes: *yes,
                shared: *shared,
                scopes: parse_scopes(scopes)?,
                via: parse_manager(via)?,
                apps: apps.clone(),
                only: vec![],
                force: false,
            };
            integrate::connect(&opts).await
        }
        Command::Doctor {
            agent,
            rebuild_derived,
        } => {
            if *rebuild_derived {
                rebuild_derived_command().await
            } else {
                integrate::doctor(parse_agent_opt(agent)?).await
            }
        }
        Command::Pattern { action } => pattern(action).await,
        Command::Repair {
            agent,
            dry_run,
            force,
        } => {
            let opts = integrate::Options {
                agent: parse_agent_opt(agent)?,
                dry_run: *dry_run,
                force: *force,
                ..Default::default()
            };
            integrate::repair(&opts).await
        }
        Command::Disconnect {
            agent,
            only,
            dry_run,
        } => {
            let opts = integrate::Options {
                only: parse_kinds(only)?,
                dry_run: *dry_run,
                ..Default::default()
            };
            integrate::disconnect(parse_agent(agent)?, &opts).await
        }
        Command::Integration { action } => integration(action).await,

        Command::Session { action } => session(action).await,
        Command::Task { action } => task(action).await,
        Command::Memory { action } => memory(action).await,
        Command::Evidence { action } => evidence(action).await,
        Command::Verify {
            memory,
            all,
            explain,
        } => {
            let v = client::send(&Request::Verify {
                cwd: cwd(),
                memory_id: *memory,
                all: *all,
                explain: *explain,
            })
            .await?;
            let text = render::verification(&v);
            Ok(Output::with(v, text))
        }

        Command::Handoff { action } => {
            let HandoffAction::Show { session } = action;
            let v = client::send(&Request::HandoffLatest {
                cwd: cwd(),
                session_id: *session,
                agent_session_key: None,
            })
            .await?;
            let h: Handoff = serde_json::from_value(v["handoff"].clone())
                .map_err(|e| WireError::invalid(e.to_string()))?;
            Ok(Output::with(v, render::handoff(&h)))
        }

        Command::Context {
            budget,
            session,
            reason,
            explain,
            depth,
        } => {
            let reason = match reason {
                Some(r) => Some(match r.as_str() {
                    "session_start" => ContextReason::SessionStart,
                    "continuation" => ContextReason::Continuation,
                    "refresh" => ContextReason::Refresh,
                    "post_compaction" => ContextReason::PostCompaction,
                    other => return Err(WireError::invalid(format!("unknown reason `{other}`"))),
                }),
                None => Some(ContextReason::Refresh),
            };
            let depth = match depth {
                Some(d) => Some(match d.as_str() {
                    "minimum" => ContextDepth::Minimum,
                    "standard" => ContextDepth::Standard,
                    other => return Err(WireError::invalid(format!("unknown depth `{other}`"))),
                }),
                None => None,
            };
            let v = client::send(&Request::Context {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                reason,
                token_budget: *budget,
                explain: *explain,
                depth,
                // The CLI always retrieves as an explicit pull (`contracts/
                // retrieval-delivery.md` §3), same as `cairn_context`.
                trigger: None,
                open_trigger: None,
            })
            .await?;
            let payload: ContextPayload =
                serde_json::from_value(v.clone()).map_err(|e| WireError::invalid(e.to_string()))?;
            // Divergence leads. A session resuming after a compaction has to
            // learn that the ground moved before it reads anything written
            // against the ground it moved from (FR-434).
            let mut text = render::continuity(&v);
            text.push_str(&render::briefing(&payload));
            if let Some(selection) = payload.selection.as_ref() {
                text.push_str(&render::selection(selection));
            }
            text.push_str(&render::continuity_footer(&v));
            Ok(Output::with(v, text))
        }

        Command::Privacy { action } => privacy(action).await,
        Command::Delete { action } => delete(action).await,

        Command::Link { project, create } => {
            let v = client::send(&Request::Link {
                cwd: cwd(),
                server_project_id: *project,
                create: *create,
            })
            .await?;
            // `Display` on a `Value` quotes strings, which put the quotes in
            // front of the reader: `Linked to shared project "019f…"`.
            let text = if v["linked"].as_bool().unwrap_or(false) {
                let id = v["server_project_id"].as_str().unwrap_or_default();
                format!("Linked to shared project {id}.\n")
            } else {
                let candidates = v["candidates"].as_array().cloned().unwrap_or_default();
                let mut t = String::from("Not linked.\n");
                if candidates.is_empty() {
                    t.push_str("No shared project matches this repository's remote.\n");
                } else {
                    t.push_str("Shared projects matching this remote:\n");
                    for c in candidates {
                        t.push_str(&format!(
                            "  {}  {}\n",
                            c["id"].as_str().unwrap_or_default(),
                            c["name"].as_str().unwrap_or_default()
                        ));
                    }
                }
                t.push_str(
                    "Run `cairn link --create` for a new shared project, \
                     or `cairn link --project <id>` to join one.\n",
                );
                t
            };
            Ok(Output::with(v, text))
        }
        Command::Unlink => {
            let v = client::send(&Request::Unlink { cwd: cwd() }).await?;
            Ok(Output::with(
                v,
                "This project is local only again.\n".into(),
            ))
        }

        Command::Auth { action } => auth(action).await,
        Command::User { action } => user(action).await,
        Command::Team { action } => team(action).await,
        Command::Personal { action } => personal(action).await,
        Command::Traits => {
            let v = client::send(&Request::ProjectTraits { cwd: cwd() }).await?;
            let traits = v["traits"].as_array().cloned().unwrap_or_default();
            let mut text = String::new();
            if traits.is_empty() {
                text.push_str("No derived traits for this project.\n");
            }
            for t in &traits {
                text.push_str(&format!(
                    "{:<10} {}\n",
                    t["kind"].as_str().unwrap_or(""),
                    t["value"].as_str().unwrap_or(""),
                ));
            }
            Ok(Output::with(v, text))
        }
        Command::Project { action } => project(action).await,
        Command::Update { check } => update_command(*check).await,
        Command::Sync { action } => sync(action).await,
        Command::Daemon { action } => daemon(action).await,
    }
}

async fn session(action: &SessionAction) -> Result<Output, WireError> {
    match action {
        SessionAction::List => {
            let v = client::send(&Request::SessionList { cwd: cwd() }).await?;
            let sessions: Vec<SessionSummary> =
                serde_json::from_value(v["sessions"].clone()).unwrap_or_default();
            let mut text = String::new();
            if sessions.is_empty() {
                text.push_str("No sessions yet.\n");
            }
            for s in &sessions {
                text.push_str(&format!(
                    "{}  {:<12} {:<14} {}  idle {}s\n",
                    s.id, s.status, s.agent, s.branch, s.idle_seconds
                ));
            }
            Ok(Output::with(v, text))
        }
        SessionAction::Start { agent, key, task } => {
            let v = client::send(&Request::SessionStart {
                cwd: cwd(),
                agent: agent.clone(),
                agent_session_key: key.clone(),
                task_id: *task,
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!("Session {} started.\n", render::id_of(&v["session"])),
            ))
        }
        SessionAction::Show { session } => {
            let v = client::send(&Request::SessionShow {
                cwd: cwd(),
                session_id: *session,
                agent_session_key: None,
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&v["session"]).unwrap_or_default()
                ),
            ))
        }
        SessionAction::Checkpoint { session } => {
            let v = client::send(&Request::SessionCheckpoint {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
            })
            .await?;
            let paths = v["checkpoint"]["relevant_paths"].as_u64().unwrap_or(0);
            Ok(Output::with(
                v,
                format!("Checkpoint recorded over {paths} relevant paths.\n"),
            ))
        }
        SessionAction::End {
            session,
            status,
            reason,
        } => {
            let status: SessionStatus = parse_enum("status", status)?;
            let v = client::send(&Request::SessionEnd {
                cwd: cwd(),
                session_id: *session,
                agent_session_key: None,
                status,
                reason: reason.clone(),
                // `cairn session end` waits for the durable handoff: nothing
                // holds a deadline over it (D22).
                wait_for_handoff: true,
            })
            .await?;
            Ok(Output::with(v, "Session ended; handoff written.\n".into()))
        }
    }
}

async fn integration(action: &IntegrationAction) -> Result<Output, WireError> {
    match action {
        IntegrationAction::Export { what } => {
            let ExportAction::Mcp { agent, format } = what;
            integrate::export_mcp(parse_agent_opt(agent)?, format.as_deref())
        }
        IntegrationAction::Migrate {
            agent,
            kind,
            to,
            dry_run,
            resume,
            abort,
        } => {
            let opts = integrate::Options {
                dry_run: *dry_run,
                ..Default::default()
            };
            integrate::migrate(
                parse_agent(agent)?,
                parse_kind(kind)?,
                cairn_integrate::model::ResourceOwner::parse(to).ok_or_else(|| {
                    WireError::invalid(format!("unknown owner `{to}`; use direct or cc-switch"))
                })?,
                &opts,
                *resume,
                *abort,
            )
            .await
        }
        IntegrationAction::Distribute {
            via,
            resource,
            apps,
            dry_run,
        } => {
            let manager = cairn_integrate::model::ManagerId::parse(via)
                .ok_or_else(|| WireError::invalid(format!("unknown manager `{via}`")))?;
            let opts = integrate::Options {
                dry_run: *dry_run,
                ..Default::default()
            };
            integrate::distribute(manager, parse_kind(resource)?, apps.clone(), &opts).await
        }
    }
}

fn parse_agent(name: &str) -> Result<cairn_integrate::model::AgentId, WireError> {
    cairn_integrate::model::AgentId::parse(name).ok_or_else(|| {
        WireError::invalid(format!(
            "unknown agent `{name}`; use claude-code, codex, opencode or generic-mcp"
        ))
    })
}

fn parse_agent_opt(
    name: &Option<String>,
) -> Result<Option<cairn_integrate::model::AgentId>, WireError> {
    match name {
        None => Ok(None),
        Some(n) => parse_agent(n).map(Some),
    }
}

fn parse_kind(name: &str) -> Result<cairn_integrate::model::ResourceKind, WireError> {
    cairn_integrate::model::ResourceKind::parse(name).ok_or_else(|| {
        WireError::invalid(format!(
            "unknown resource kind `{name}`; use mcp, lifecycle, instructions or skill"
        ))
    })
}

fn parse_kinds(names: &[String]) -> Result<Vec<cairn_integrate::model::ResourceKind>, WireError> {
    names.iter().map(|n| parse_kind(n)).collect()
}

/// `--scope <kind>=<scope>`, repeatable.
fn parse_scopes(
    raw: &[String],
) -> Result<
    Vec<(
        cairn_integrate::model::ResourceKind,
        cairn_integrate::model::InstallationScope,
    )>,
    WireError,
> {
    raw.iter()
        .map(|s| {
            let (kind, scope) = s.split_once('=').ok_or_else(|| {
                WireError::invalid(format!("--scope wants <kind>=<scope>, got `{s}`"))
            })?;
            let kind = parse_kind(kind)?;
            let scope =
                cairn_integrate::model::InstallationScope::parse(scope).ok_or_else(|| {
                    WireError::invalid(format!(
                        "unknown scope `{scope}`; use project_shared, project_local or user"
                    ))
                })?;
            Ok((kind, scope))
        })
        .collect()
}

fn parse_manager(
    raw: &Option<String>,
) -> Result<Option<cairn_integrate::model::ManagerId>, WireError> {
    match raw {
        None => Ok(None),
        Some(v) => cairn_integrate::model::ManagerId::parse(v)
            .map(Some)
            .ok_or_else(|| WireError::invalid(format!("unknown manager `{v}`"))),
    }
}

async fn task(action: &TaskAction) -> Result<Output, WireError> {
    match action {
        TaskAction::List { status } => {
            let status = match status {
                Some(s) => Some(parse_enum::<TaskStatus>("status", s)?),
                None => None,
            };
            let v = client::send(&Request::TaskList { cwd: cwd(), status }).await?;
            let tasks: Vec<Task> = serde_json::from_value(v["tasks"].clone()).unwrap_or_default();
            let mut text = String::new();
            if tasks.is_empty() {
                text.push_str("No tasks yet.\n");
            }
            for t in &tasks {
                text.push_str(&format!("{}  {:<12} {}\n", t.id, t.status, t.title));
            }
            Ok(Output::with(v, text))
        }
        TaskAction::Show { id } => {
            let v = client::send(&Request::TaskGet {
                cwd: cwd(),
                task_id: *id,
            })
            .await?;
            let t: Task = serde_json::from_value(v["task"].clone())
                .map_err(|e| WireError::invalid(e.to_string()))?;
            let mut text = format!("{}\n{}\nStatus: {}\n", t.title, t.goal, t.status);
            text.push_str(&render::task_work_state(&v));
            Ok(Output::with(v, text))
        }
        TaskAction::New {
            title,
            goal,
            criteria,
        } => {
            let v = client::send(&Request::TaskCreate {
                cwd: cwd(),
                title: title.clone(),
                goal: goal.clone(),
                acceptance_criteria: criteria.clone(),
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!("Task {} created.\n", render::id_of(&v["task"])),
            ))
        }
        TaskAction::SetStatus { id, status } => {
            let status: TaskStatus = parse_enum("status", status)?;
            let v = client::send(&Request::TaskUpdate {
                cwd: cwd(),
                task_id: *id,
                title: None,
                goal: None,
                acceptance_criteria: None,
                status: Some(status),
            })
            .await?;
            Ok(Output::with(v, format!("Task is now {status}.\n")))
        }
        TaskAction::Update {
            id,
            title,
            goal,
            acceptance_criteria,
            status,
        } => {
            let status: Option<TaskStatus> = match status {
                Some(s) => Some(parse_enum("status", s)?),
                None => None,
            };
            let v = client::send(&Request::TaskUpdate {
                cwd: cwd(),
                task_id: *id,
                title: title.clone(),
                goal: goal.clone(),
                acceptance_criteria: acceptance_criteria.clone(),
                status,
            })
            .await?;
            Ok(Output::with(v, "Task updated.\n".to_string()))
        }
        TaskAction::Criterion(action) => criterion(action).await,
        TaskAction::Blocker(action) => blocker(action).await,
        TaskAction::Readiness { id } => {
            let v = client::send(&Request::TaskReadiness {
                cwd: cwd(),
                task_id: *id,
            })
            .await?;
            let text = render::readiness(&v);
            Ok(Output::with(v, text))
        }
        TaskAction::History { id, limit } => {
            let v = client::send(&Request::TaskHistory {
                cwd: cwd(),
                task_id: *id,
                limit: *limit,
            })
            .await?;
            let text = render::task_history(&v);
            Ok(Output::with(v, text))
        }
    }
}

async fn criterion(action: &CriterionAction) -> Result<Output, WireError> {
    match action {
        CriterionAction::Add {
            task_id,
            text,
            session,
        } => {
            let v = client::send(&Request::TaskCriterionAdd {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                task_id: *task_id,
                text: text.clone(),
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!(
                    "{} added.\n",
                    v["criterion"]["label"].as_str().unwrap_or("?")
                ),
            ))
        }
        CriterionAction::Set {
            criterion_id,
            state,
            text,
            expected_revision,
            session,
        } => {
            let state: Option<CriterionState> = match state {
                Some(s) => Some(parse_enum("state", s)?),
                None => None,
            };
            let v = client::send(&Request::TaskCriterionSet {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                criterion_id: *criterion_id,
                state,
                text: text.clone(),
                expected_revision: *expected_revision,
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                render::criterion_line(&v["criterion"]),
            ))
        }
        CriterionAction::Verify {
            criterion_id,
            evidence,
            session,
        } => {
            let v = client::send(&Request::TaskCriterionVerify {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                criterion_id: *criterion_id,
                evidence_id: *evidence,
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                render::criterion_line(&v["criterion"]),
            ))
        }
        CriterionAction::Remove {
            criterion_id,
            session,
        } => {
            let v = client::send(&Request::TaskCriterionRemove {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                criterion_id: *criterion_id,
            })
            .await?;
            Ok(Output::with(
                v,
                "Criterion removed. Ordinals are not renumbered.\n".to_string(),
            ))
        }
    }
}

async fn blocker(action: &BlockerAction) -> Result<Output, WireError> {
    match action {
        BlockerAction::Open {
            task_id,
            description,
            session,
        } => {
            let v = client::send(&Request::TaskBlockerOpen {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                task_id: *task_id,
                description: description.clone(),
            })
            .await?;
            Ok(Output::with(v, "Blocker opened.\n".to_string()))
        }
        BlockerAction::Clear {
            blocker_id,
            session,
        } => {
            let v = client::send(&Request::TaskBlockerClear {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                blocker_id: *blocker_id,
            })
            .await?;
            Ok(Output::with(v, "Blocker cleared.\n".to_string()))
        }
    }
}

async fn evidence(action: &EvidenceAction) -> Result<Output, WireError> {
    match action {
        EvidenceAction::Add {
            kind,
            subject,
            value,
            locator,
            collector,
            observation,
            memory,
            role,
            session,
        } => {
            let v = client::send(&Request::EvidenceAdd {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                kind: parse_enum("type", kind)?,
                collector: match collector {
                    Some(c) => Some(parse_enum("collector", c)?),
                    None => None,
                },
                subject: subject.clone(),
                observed_value: value.clone(),
                source_locator: locator.clone(),
                observation_id: *observation,
                memory_id: *memory,
                role: match role {
                    Some(r) => Some(parse_enum("role", r)?),
                    None => None,
                },
            })
            .await?;
            // Who collected it, named at the moment it is recorded. Whether
            // Cairn read the value or an agent asserted it is what decides
            // the authority a later verification may claim (FR-370), and a
            // bare "recorded" leaves the writer unable to tell which they
            // just created.
            let text = format!(
                "Evidence {} recorded.  collector: {}\n",
                render::id_of(&v["evidence"]),
                v["evidence"]["collector"].as_str().unwrap_or("?")
            );
            Ok(Output::with(v, text))
        }
        EvidenceAction::List { memory } => {
            let v = client::send(&Request::EvidenceList {
                cwd: cwd(),
                memory_id: *memory,
            })
            .await?;
            let text = render::evidence_list(&v);
            Ok(Output::with(v, text))
        }
        EvidenceAction::Show { id } => {
            let v = client::send(&Request::EvidenceShow {
                cwd: cwd(),
                evidence_id: *id,
            })
            .await?;
            let text = render::evidence_list(&v);
            Ok(Output::with(v, text))
        }
    }
}

async fn memory(action: &MemoryAction) -> Result<Output, WireError> {
    match action {
        MemoryAction::Pin {
            id,
            off,
            reason,
            session,
        } => {
            let v = client::send(&Request::MemoryPin {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                memory_id: *id,
                pinned: !*off,
                reason: reason.clone(),
            })
            .await?;
            Ok(Output::with(
                v,
                if *off {
                    "Unpinned.\n".to_string()
                } else {
                    "Pinned. It now leads every briefing in this scope.\n".to_string()
                },
            ))
        }
        MemoryAction::Add {
            content,
            kind,
            scope,
            scope_key,
            local_only,
            evidence,
            topic_key,
            value_key,
            importance,
            session,
        } => {
            let kind: MemoryType = parse_enum("type", kind)?;
            let importance = match importance {
                Some(i) => Some(parse_enum::<Importance>("importance", i)?),
                None => None,
            };
            let scope = match scope {
                Some(s) => Some(parse_enum::<MemoryScope>("scope", s)?),
                None => None,
            };
            let v = client::send(&Request::MemoryCreate {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                kind,
                scope,
                scope_key: scope_key.clone(),
                content: content.clone(),
                topic_key: topic_key.clone(),
                value_key: value_key.clone(),
                importance,
                evidence_observation_ids: evidence.clone(),
                local_only: *local_only,

                domain: None,
            })
            .await?;
            let text = format!(
                "Remembered {}.\n{}",
                render::id_of(&v["memory"]),
                render::reconciliation(&v)
            );
            Ok(Output::with(v, text))
        }
        MemoryAction::Supersede {
            content,
            memory_id,
            kind,
            scope,
            scope_key,
            local_only,
            evidence,
            topic_key,
            value_key,
            importance,
            session,
        } => {
            let kind: MemoryType = parse_enum("type", kind)?;
            let importance = match importance {
                Some(i) => Some(parse_enum::<Importance>("importance", i)?),
                None => None,
            };
            let scope = match scope {
                Some(s) => Some(parse_enum::<MemoryScope>("scope", s)?),
                None => None,
            };
            let v = client::send(&Request::MemorySupersede {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                memory_id: *memory_id,
                kind,
                scope,
                scope_key: scope_key.clone(),
                content: content.clone(),
                topic_key: topic_key.clone(),
                value_key: value_key.clone(),
                importance,
                evidence_observation_ids: evidence.clone(),
                local_only: *local_only,
            })
            .await?;
            let text = format!(
                "Remembered {}.\n  supersedes {}\n",
                render::id_of(&v["memory"]),
                v["superseded"].as_str().unwrap_or("?")
            );
            Ok(Output::with(v, text))
        }
        MemoryAction::Subject {
            topic_key,
            scope,
            scope_key,
            domain,
        } => {
            let scope = match scope {
                Some(s) => Some(parse_enum::<MemoryScope>("scope", s)?),
                None => None,
            };
            let domain = match domain {
                Some(d) => Some(parse_enum::<KnowledgeDomain>("domain", d)?),
                None => None,
            };
            if domain.is_some_and(|d| d != KnowledgeDomain::Project)
                && (scope.is_some() || scope_key.is_some())
            {
                return Err(WireError::invalid(
                    "--scope and --scope-key apply to project memory only; personal and \
                     team knowledge have no scope",
                ));
            }
            let v = client::send(&Request::MemorySubject {
                cwd: cwd(),
                topic_key: topic_key.clone(),
                scope,
                scope_key: scope_key.clone(),
                domain,
            })
            .await?;
            let text = render::subject(&v);
            Ok(Output::with(v, text))
        }
        MemoryAction::Reinforce { id, from, session } => {
            let v = client::send(&Request::MemoryReinforce {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                memory_id: *id,
                from_memory_id: Some(*from),
            })
            .await?;
            let counts = format!(
                "Reinforced. reinforcements {} · distinct origins {}\n",
                v.get("reinforcements")
                    .and_then(|n| n.as_i64())
                    .unwrap_or(0),
                v.get("distinct_origins")
                    .and_then(|n| n.as_i64())
                    .unwrap_or(1),
            );
            Ok(Output::with(v, counts))
        }
        MemoryAction::Reconcile {
            from,
            to,
            relation,
            basis,
            basis_evidence,
            rationale,
            session,
        } => {
            let v = client::send(&Request::MemoryReconcile {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                from_memory_id: *from,
                to_memory_id: *to,
                relation: parse_enum("relation", relation)?,
                basis: parse_enum("basis", basis)?,
                basis_evidence_id: *basis_evidence,
                rationale: rationale.clone(),
            })
            .await?;
            Ok(Output::with(v, "Decision recorded.\n".to_string()))
        }
        MemoryAction::Search {
            query,
            scope,
            scope_key,
            kind,
            state,
            limit,
            topic_key,
            as_of,
            conflicted,
            corroborated,
            include_patterns,
            verification,
            authority,
            session,
        } => {
            let as_of = match as_of {
                Some(t) => Some(
                    chrono::DateTime::parse_from_rfc3339(t)
                        .map_err(|e| {
                            WireError::invalid(format!("--as-of must be an RFC 3339 instant: {e}"))
                        })?
                        .with_timezone(&chrono::Utc),
                ),
                None => None,
            };
            let q = MemoryQuery {
                query: query.clone(),
                scope: match scope {
                    Some(s) => Some(parse_enum("scope", s)?),
                    None => None,
                },
                scope_key: scope_key.clone(),
                kind: match kind {
                    Some(k) => Some(parse_enum("type", k)?),
                    None => None,
                },
                state: match state {
                    Some(s) => Some(parse_enum("state", s)?),
                    None => None,
                },
                limit: *limit,
                topic_key: topic_key.clone(),
                as_of,
                conflicted: *conflicted,
                corroborated: *corroborated,
                include_patterns: *include_patterns,
                verification: match verification {
                    Some(v) => Some(parse_enum("verification", v)?),
                    None => None,
                },
                authority: match authority {
                    Some(a) => Some(parse_enum("authority", a)?),
                    None => None,
                },

                domains: None,
            };
            let v = client::send(&Request::MemorySearch {
                cwd: cwd(),
                agent_session_key: None,
                session_id: *session,
                query: q,
            })
            .await?;
            let payload: SearchPayload =
                serde_json::from_value(v.clone()).map_err(|e| WireError::invalid(e.to_string()))?;
            let mut text = String::new();
            // A historical answer, echoed back so it cannot be mistaken for a
            // current one (FR-342, D82). Without this line a `--as-of` result
            // and a live one are the same eight lines of output.
            if let Some(instant) = as_of {
                // RFC 3339, the same spelling the flag takes, so the line can be
                // pasted back into the command that produced it.
                text.push_str(&format!(
                    "as_of {} — what this project believed then, not now\n\n",
                    instant.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                ));
            }
            if payload.results.is_empty() {
                text.push_str("No matching memory.\n");
            }
            for r in &payload.results {
                text.push_str(&render::search_result(r));
            }
            Ok(Output::with(v, text))
        }
        MemoryAction::Show { id } => {
            let v = client::send(&Request::MemoryGet {
                cwd: cwd(),
                memory_id: *id,
            })
            .await?;
            let text =
                match serde_json::from_value::<cairn_core::wire::MemoryResult>(v["memory"].clone())
                {
                    Ok(m) => render::memory_detail(&m),
                    // A shape this build does not know is still worth showing, and
                    // showing it raw is better than showing nothing.
                    Err(_) => format!(
                        "{}\n",
                        serde_json::to_string_pretty(&v["memory"]).unwrap_or_default()
                    ),
                };
            Ok(Output::with(v, text))
        }
        MemoryAction::Forget { id } => {
            let v = client::send(&Request::MemoryForget {
                cwd: cwd(),
                memory_id: *id,

                domain: None,
            })
            .await?;
            Ok(Output::with(v, "Memory deleted.\n".into()))
        }
    }
}

async fn privacy(action: &PrivacyAction) -> Result<Output, WireError> {
    let request = match action {
        PrivacyAction::Exclude { path, command } => Request::PrivacyExclude {
            cwd: cwd(),
            path: path.clone(),
            command: command.clone(),
        },
        PrivacyAction::Unexclude { path, command } => Request::PrivacyUnexclude {
            cwd: cwd(),
            path: path.clone(),
            command: command.clone(),
        },
        PrivacyAction::List => Request::PrivacyList { cwd: cwd() },
    };
    let v = client::send(&request).await?;
    let mut text = String::from("Excluded paths:\n");
    for p in v["paths"].as_array().cloned().unwrap_or_default() {
        text.push_str(&format!("  {}\n", p.as_str().unwrap_or_default()));
    }
    text.push_str("Excluded commands:\n");
    for c in v["commands"].as_array().cloned().unwrap_or_default() {
        text.push_str(&format!("  {}\n", c.as_str().unwrap_or_default()));
    }
    Ok(Output::with(v, text))
}

async fn delete(action: &DeleteAction) -> Result<Output, WireError> {
    let (target, id, with_memories) = match action {
        DeleteAction::Observation { id } => (DeleteTarget::Observation, *id, false),
        DeleteAction::Memory { id } => (DeleteTarget::Memory, *id, false),
        DeleteAction::Handoff { id } => (DeleteTarget::Handoff, *id, false),
        DeleteAction::Session { id, with_memories } => (DeleteTarget::Session, *id, *with_memories),
    };
    let v = client::send(&Request::Delete {
        cwd: cwd(),
        target,
        id,
        with_memories,
    })
    .await?;
    let mut text = format!("Deleted {id}.\n");
    if target == DeleteTarget::Session && !with_memories {
        text.push_str(
            "The memories and handoffs this session produced were kept; \
             pass --with-memories to remove them too.\n",
        );
    }
    Ok(Output::with(v, text))
}

async fn auth(action: &AuthAction) -> Result<Output, WireError> {
    match action {
        AuthAction::Token {
            action: TokenAction::Set { token, server },
        } => {
            let token = match token {
                Some(t) => t.clone(),
                None => {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map_err(|e| WireError::invalid(e.to_string()))?;
                    buf.trim().to_string()
                }
            };
            if token.is_empty() {
                return Err(WireError::invalid("no token supplied"));
            }
            let v = client::send(&Request::AuthTokenSet {
                token,
                server_url: server.clone(),
            })
            .await?;
            Ok(Output::with(v, "Token stored.\n".into()))
        }
        AuthAction::Logout => {
            let v = client::send(&Request::AuthLogout).await?;
            Ok(Output::with(v, "Token removed.\n".into()))
        }
        AuthAction::Status => {
            let v = client::send(&Request::AuthStatus).await?;
            let authenticated = v
                .get("authenticated")
                .and_then(|b| b.as_bool())
                .unwrap_or(false);
            let server = v
                .get("server_url")
                .and_then(|u| u.as_str())
                .unwrap_or("not set");
            let text = format!(
                "Token   {}\nServer  {server}\n",
                if authenticated { "stored" } else { "none" },
            );
            Ok(Output::with(v, text))
        }
        AuthAction::ChangePassword { new_password } => {
            let new_password = match new_password {
                Some(p) => p.clone(),
                None => prompt_line("New password: ")?,
            };
            if new_password.is_empty() {
                return Err(WireError::invalid("no new password supplied"));
            }
            let v = client::send(&Request::AuthChangePassword { new_password }).await?;
            Ok(Output::with(
                v,
                "Password changed. Every existing web session for this account was ended; \
                 API tokens you already minted are unaffected.\n"
                    .into(),
            ))
        }
    }
}

/// A single line read from stdin, with the prompt written to stderr so it
/// never lands in `--json` output. Not masked — this crate carries no
/// terminal-echo dependency — the same tradeoff `auth token set` already
/// makes for the token itself.
fn prompt_line(prompt: &str) -> Result<String, WireError> {
    use std::io::Write;
    eprint!("{prompt}");
    std::io::stderr()
        .flush()
        .map_err(|e| WireError::invalid(e.to_string()))?;
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .map_err(|e| WireError::invalid(e.to_string()))?;
    Ok(buf.trim().to_string())
}

/// `cairn user` (`contracts/identity-administration.md` §9). Every
/// subcommand is daemon-mediated like `link`/`auth` above — the daemon holds
/// the token and makes the HTTP call, never this process.
async fn user(action: &UserAction) -> Result<Output, WireError> {
    match action {
        UserAction::Create {
            email,
            display_name,
        } => {
            let v = client::send(&Request::AdminUserCreate {
                email: email.clone(),
                display_name: display_name.clone(),
            })
            .await?;
            let email = v.get("email").and_then(|e| e.as_str()).unwrap_or(email);
            let temporary = v
                .get("temporary_password")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let text = format!(
                "Created {email}. Temporary password: {temporary}\n\
                 This password is shown once, right here, and is never stored anywhere it \
                 can be read back — not even by the administrator who ran this command. If \
                 it is lost before the account's first sign-in, the remedy is \
                 `cairn user reset-password {email}`.\n\
                 They must change it before doing anything else.\n"
            );
            Ok(Output::with(v, text))
        }

        UserAction::List => {
            let v = client::send(&Request::AdminUserList).await?;
            let users = v["users"].as_array().cloned().unwrap_or_default();
            let mut text = format!(
                "{:<32} {:<7} {:<9} {:<21} {}\n",
                "EMAIL", "ROLE", "STATUS", "MUST_CHANGE_PASSWORD", "CREATED_AT"
            );
            if users.is_empty() {
                text.push_str("no accounts\n");
            }
            for u in &users {
                text.push_str(&format!(
                    "{:<32} {:<7} {:<9} {:<21} {}\n",
                    u["email"].as_str().unwrap_or(""),
                    u["role"].as_str().unwrap_or(""),
                    u["status"].as_str().unwrap_or(""),
                    u["must_change_password"]
                        .as_bool()
                        .unwrap_or(false)
                        .to_string(),
                    u["created_at"].as_str().unwrap_or(""),
                ));
            }
            Ok(Output::with(v, text))
        }

        UserAction::Disable { email } => {
            let v = client::send(&Request::AdminUserPatch {
                email: email.clone(),
                role: None,
                status: Some(UserStatus::Disabled),
            })
            .await?;
            Ok(Output::with(
                v,
                format!(
                    "Disabled {email}. Its active API tokens were revoked in the same \
                     transaction and stop working immediately.\n"
                ),
            ))
        }
        UserAction::Enable { email } => {
            let v = client::send(&Request::AdminUserPatch {
                email: email.clone(),
                role: None,
                status: Some(UserStatus::Active),
            })
            .await?;
            Ok(Output::with(
                v,
                format!("Enabled {email}. Existing tokens remain revoked; mint a new one.\n"),
            ))
        }
        UserAction::Promote { email } => {
            let v = client::send(&Request::AdminUserPatch {
                email: email.clone(),
                role: Some(ServerRole::Admin),
                status: None,
            })
            .await?;
            Ok(Output::with(v, format!("{email} is now admin.\n")))
        }
        UserAction::Demote { email } => {
            let v = client::send(&Request::AdminUserPatch {
                email: email.clone(),
                role: Some(ServerRole::Member),
                status: None,
            })
            .await?;
            Ok(Output::with(v, format!("{email} is now member.\n")))
        }

        UserAction::ResetPassword { email } => {
            let v = client::send(&Request::ResetPassword {
                email: email.clone(),
            })
            .await?;
            let temporary = v
                .get("temporary_password")
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let disabled = v.get("status").and_then(|s| s.as_str()) == Some("disabled");
            let mut text = format!(
                "Reset {email}. Temporary password: {temporary}\n\
                 This password is shown once, right here, and cannot be retrieved again. \
                 Every existing token this account held was revoked.\n"
            );
            if disabled {
                text.push_str(
                    "The account remains disabled and cannot authenticate until an \
                     administrator re-enables it with `cairn user enable`.\n",
                );
            } else {
                text.push_str("They must change it before doing anything else.\n");
            }
            Ok(Output::with(v, text))
        }
    }
}

/// `cairn team` (`contracts/global-memory.md` §5b, T133).
async fn team(action: &TeamAction) -> Result<Output, WireError> {
    match action {
        TeamAction::List { all } => {
            let v = client::send(&Request::TeamList { all: *all }).await?;
            let entries = v["entries"].as_array().cloned().unwrap_or_default();
            let mut text = format!(
                "{:<38} {:<13} {:<16} {}\n",
                "ID", "STATE", "TOPIC", "CONTENT"
            );
            if entries.is_empty() {
                text.push_str("no entries\n");
            }
            for e in &entries {
                text.push_str(&format!(
                    "{:<38} {:<13} {:<16} {}\n",
                    e["id"].as_str().unwrap_or(""),
                    e["state"].as_str().unwrap_or(""),
                    e["topic_key"].as_str().unwrap_or(""),
                    e["content"].as_str().unwrap_or(""),
                ));
            }
            Ok(Output::with(v, text))
        }
        TeamAction::Propose {
            content,
            kind,
            topic_key,
            value_key,
            applies_to,
        } => {
            let v = client::send(&Request::TeamPropose {
                cwd: cwd(),
                content: content.clone(),
                knowledge_type: Some(parse_enum::<MemoryType>("type", kind)?),
                topic_key: topic_key.clone(),
                value_key: value_key.clone(),
                applicability: applies_to.clone(),
            })
            .await?;
            Ok(Output::with(
                v.clone(),
                format!("proposed {}\n", v["entry"]["id"].as_str().unwrap_or("")),
            ))
        }
        TeamAction::Ratify { id, supersedes } => {
            let v = client::send(&Request::TeamRatify {
                id: *id,
                supersedes: *supersedes,
            })
            .await?;
            Ok(Output::with(v, format!("ratified {id}\n")))
        }
        TeamAction::Retire { id } => {
            let v = client::send(&Request::TeamRetire { id: *id }).await?;
            Ok(Output::with(v, format!("retired {id}\n")))
        }
    }
}

/// `cairn personal` — this account's own personal knowledge (T082).
async fn personal(action: &PersonalAction) -> Result<Output, WireError> {
    match action {
        PersonalAction::List { query, limit } => {
            let v = client::send(&Request::PersonalList {
                query: query.clone(),
                limit: *limit,
            })
            .await?;
            let entries = v["entries"].as_array().cloned().unwrap_or_default();
            let mut text = format!("{:<38} {:<16} {}\n", "ID", "TOPIC", "CONTENT");
            if entries.is_empty() {
                text.push_str("no entries\n");
            }
            for e in &entries {
                text.push_str(&format!(
                    "{:<38} {:<16} {}\n",
                    e["id"].as_str().unwrap_or(""),
                    e["topic_key"].as_str().unwrap_or(""),
                    e["content"].as_str().unwrap_or(""),
                ));
            }
            Ok(Output::with(v, text))
        }
        PersonalAction::Forget { id } => {
            let v = client::send(&Request::PersonalForget { id: *id }).await?;
            Ok(Output::with(v, format!("forgotten {id}\n")))
        }
    }
}

/// `cairn project member` (T063).
async fn project(action: &ProjectAction) -> Result<Output, WireError> {
    let ProjectAction::Member { action } = action;
    match action {
        MemberAction::Add { project_id, email } => {
            let v = client::send(&Request::ProjectMemberAdd {
                project_id: *project_id,
                email: email.clone(),
            })
            .await?;
            Ok(Output::with(v, format!("added {email} to {project_id}\n")))
        }
        MemberAction::Remove { project_id, email } => {
            let v = client::send(&Request::ProjectMemberRemove {
                project_id: *project_id,
                email: email.clone(),
            })
            .await?;
            Ok(Output::with(
                v,
                format!("removed {email} from {project_id}\n"),
            ))
        }
        MemberAction::List { project_id } => {
            let v = client::send(&Request::ProjectMemberList {
                project_id: *project_id,
            })
            .await?;
            let members = v["members"].as_array().cloned().unwrap_or_default();
            let mut text = format!("{:<32} {}\n", "EMAIL", "DISPLAY_NAME");
            if members.is_empty() {
                text.push_str("no members\n");
            }
            for m in &members {
                text.push_str(&format!(
                    "{:<32} {}\n",
                    m["email"].as_str().unwrap_or(""),
                    m["display_name"].as_str().unwrap_or(""),
                ));
            }
            Ok(Output::with(v, text))
        }
    }
}

/// `cairn doctor --rebuild-derived` (FR-478, FR-518, SC-324).
///
/// Every derived value recomputed from the records behind it, and how many
/// disagreed. A difference is a bug report rather than a normal outcome, so a
/// non-zero count is a non-zero exit — this is a release gate, and a gate that
/// reports a problem and exits 0 is not one.
async fn rebuild_derived_command() -> Result<Output, WireError> {
    let v = client::send(&Request::RebuildDerived { cwd: cwd() }).await?;

    let mut text = String::new();
    for outcome in v["derived"].as_array().unwrap_or(&Vec::new()) {
        text.push_str(&format!(
            "{:<34} {:>6} checked  {:>4} differed\n",
            outcome["derived"].as_str().unwrap_or(""),
            outcome["checked"].as_i64().unwrap_or(0),
            outcome["differed"].as_i64().unwrap_or(0),
        ));
    }
    let differed = v["differed"].as_i64().unwrap_or(0);
    text.push_str(&if differed == 0 {
        "\nevery derived value equals its rebuild\n".to_string()
    } else {
        format!("\n{differed} derived value(s) disagree with their rebuild\n")
    });

    if differed > 0 {
        return Err(WireError::new(
            "derived_inconsistent",
            format!("{differed} derived value(s) disagree with their rebuild"),
        ));
    }
    Ok(Output::with(v, text))
}

/// `cairn pattern …` (`contracts/patterns.md` §Surfaces).
async fn pattern(action: &PatternAction) -> Result<Output, WireError> {
    match action {
        PatternAction::List { trust, signal } => {
            let trust = match trust {
                Some(t) => Some(parse_enum::<PatternTrust>("trust", t)?),
                None => None,
            };
            let v = client::send(&Request::PatternList {
                cwd: cwd(),
                trust,
                signal: signal.clone(),
            })
            .await?;
            let mut text = String::new();
            for p in v["patterns"].as_array().unwrap_or(&Vec::new()) {
                text.push_str(&format!(
                    "{}  {}\n  trust {} · {}\n",
                    p["id"].as_str().unwrap_or(""),
                    p["title"].as_str().unwrap_or(""),
                    p["trust"].as_str().unwrap_or(""),
                    p["counts"].as_str().unwrap_or("")
                ));
            }
            if text.is_empty() {
                text.push_str("no patterns\n");
            }
            Ok(Output::with(v, text))
        }
        PatternAction::Show { id } => {
            let v = client::send(&Request::PatternShow {
                cwd: cwd(),
                id: *id,
            })
            .await?;
            let p = &v["pattern"];
            let mut text = format!(
                "{}\n  trust {} · unverified in any project but where it was applied\n  {}\n",
                p["title"].as_str().unwrap_or(""),
                p["trust"].as_str().unwrap_or(""),
                v["counts"].as_str().unwrap_or("")
            );
            text.push_str(&format!(
                "  problem   {}\n  cause     {}\n  approach  {}\n",
                p["problem"].as_str().unwrap_or(""),
                p["root_cause"].as_str().unwrap_or(""),
                p["approach"].as_str().unwrap_or("")
            ));
            for c in v["alternative_causes"].as_array().unwrap_or(&Vec::new()) {
                text.push_str(&format!(
                    "  ⚠ known alternative cause: {}\n",
                    c.as_str().unwrap_or("")
                ));
            }
            Ok(Output::with(v, text))
        }
        PatternAction::Promote {
            memory,
            signals,
            title,
            problem,
            applicability,
            root_cause,
            approach,
            constraints,
            dry_run,
        } => {
            let v = client::send(&Request::PatternPromote {
                cwd: cwd(),
                memory_id: *memory,
                title: title.clone(),
                problem: problem.clone(),
                signals: signals.clone(),
                applicability: applicability.clone(),
                root_cause: root_cause.clone(),
                approach: approach.clone(),
                constraints: constraints.clone(),
                dry_run: *dry_run,

                target: None,
                applicability_facts: Vec::new(),
            })
            .await?;
            let text = if *dry_run {
                format!(
                    "would promote: {}\n(nothing was written)\n",
                    v["pattern"]["title"].as_str().unwrap_or("")
                )
            } else {
                format!(
                    "promoted {}\n  {}\n",
                    v["pattern"]["id"].as_str().unwrap_or(""),
                    v["pattern"]["title"].as_str().unwrap_or("")
                )
            };
            Ok(Output::with(v, text))
        }
        PatternAction::Outcome {
            id,
            outcome,
            signals,
            alternative_cause,
            evidence,
            session,
        } => {
            let v = client::send(&Request::PatternOutcome {
                cwd: cwd(),
                id: *id,
                outcome: parse_enum::<PatternOutcome>("outcome", outcome)?,
                signals: signals.clone(),
                alternative_cause: alternative_cause.clone(),
                evidence_id: *evidence,
                session: *session,
            })
            .await?;
            let text = format!(
                "recorded {} ({})\n  trust {} · {}\n",
                v["outcome"].as_str().unwrap_or(""),
                v["discovery"].as_str().unwrap_or(""),
                v["trust"].as_str().unwrap_or(""),
                v["counts"].as_str().unwrap_or("")
            );
            Ok(Output::with(v, text))
        }
        PatternAction::Forget { id } => {
            let v = client::send(&Request::PatternForget {
                cwd: cwd(),
                id: *id,
            })
            .await?;
            Ok(Output::with(v, format!("forgotten {id}\n")))
        }
    }
}

async fn sync(action: &SyncAction) -> Result<Output, WireError> {
    match action {
        SyncAction::Status => {
            let v = client::send(&Request::SyncStatus { cwd: cwd() }).await?;
            let s: SyncStatusPayload =
                serde_json::from_value(v.clone()).map_err(|e| WireError::invalid(e.to_string()))?;
            let mut text = format!(
                "Linked       {}\nPending      {}\nFailed       {}\nLast success {}\n",
                if s.linked { "yes" } else { "no" },
                s.pending,
                s.failed,
                s.last_success_at
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "never".into())
            );
            for f in &s.failures {
                text.push_str(&format!(
                    "  failed {} {}: {}\n",
                    f.entity_type, f.entity_id, f.error
                ));
            }
            // Retained work is reported on its own line, never folded into
            // `Failed`: it is waiting, not lost, and saying so is the whole
            // point of the state (FR-415).
            if let Some(d) = &s.degradation {
                text.push_str(&format!(
                    "Blocked      {} (waiting for: {})\n  server: {}\n  {}\n",
                    d.blocked,
                    d.missing_capabilities.join(", "),
                    d.server_capability,
                    d.note
                ));
            }
            // Per-namespace breakdown (T109, FR-487): `project:*` is always
            // present; `personal:*`/`team:*` appear only once this store has
            // ever queued something in that namespace.
            if let Some(namespaces) = v.get("namespaces").and_then(|n| n.as_array()) {
                text.push_str("Namespaces\n");
                for n in namespaces {
                    let key = n["namespace"].as_str().unwrap_or("");
                    let kind = n["kind"].as_str().unwrap_or("");
                    let pending = n["pending"].as_i64().unwrap_or(0);
                    let failed = n["failed"].as_i64().unwrap_or(0);
                    let blocked = n["blocked"].as_i64().unwrap_or(0);
                    // Derived, not carried on the payload: a namespace has no
                    // `state` field of its own, only these three counts —
                    // this is the same "worst first" ordering `SyncStatusPayload`
                    // already gives `Blocked`/`Failed`/`Pending` above.
                    let state = if blocked > 0 {
                        "blocked"
                    } else if failed > 0 {
                        "failed"
                    } else if pending > 0 {
                        "pending"
                    } else {
                        "current"
                    };
                    text.push_str(&format!(
                        "  {key:<32} {state:<8} pending={pending} failed={failed} blocked={blocked}\n"
                    ));
                    // The one degradation reason this payload carries is
                    // project-scoped (`SyncDegradation` above); a `personal:*`
                    // or `team:*` namespace's own blocked reason is not on
                    // this payload yet, so it is only ever attributed to the
                    // namespace it is actually about, never guessed for
                    // another.
                    if blocked > 0 && kind == "project" {
                        if let Some(d) = &s.degradation {
                            text.push_str(&format!("    {}\n", d.note));
                        }
                    }
                    // Holes in a writer's own sequence (FR-492, SC-450).
                    // Reported here and nowhere else, because a gap nobody
                    // surfaces is indistinguishable from a stream that had
                    // none — and this is the whole reason `writer_seq` travels
                    // at all. Diagnostic: nothing acts on it, and the operator
                    // reading it is the point.
                    for gap in n["gaps"].as_array().unwrap_or(&Vec::new()) {
                        let missing: Vec<String> = gap["missing"]
                            .as_array()
                            .unwrap_or(&Vec::new())
                            .iter()
                            .map(|v| v.to_string())
                            .collect();
                        if missing.is_empty() {
                            continue;
                        }
                        text.push_str(&format!(
                            "    gap: writer {} is missing {} of {} (never arrived: {})\n",
                            gap["writer_id"].as_str().unwrap_or("?"),
                            missing.len(),
                            gap["highest_seen"].as_i64().unwrap_or(0),
                            missing.join(", ")
                        ));
                    }
                }
            }
            Ok(Output::with(v, text))
        }
        SyncAction::Now => {
            let v = client::send(&Request::SyncNow { cwd: cwd() }).await?;
            Ok(Output::with(
                v.clone(),
                format!(
                    "applied {}, duplicate {}, rejected {}, pulled {}\n",
                    v["applied"], v["duplicate"], v["rejected"], v["pulled"]
                ),
            ))
        }
    }
}

async fn daemon(action: &DaemonAction) -> Result<Output, WireError> {
    match action {
        DaemonAction::Logs { tail } => {
            let path = cairn_core::paths::daemon_log_path();
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let lines: Vec<&str> = text.lines().collect();
            let shown: Vec<&str> = lines.iter().rev().take(*tail).rev().copied().collect();
            let body = if shown.is_empty() {
                format!(
                    "No daemon log yet at {}.\nIt fills once the daemon starts.\n",
                    path.display()
                )
            } else {
                format!("{}\n", shown.join("\n"))
            };
            Ok(Output::with(
                serde_json::json!({
                    "path": path.display().to_string(),
                    "lines": shown,
                }),
                body,
            ))
        }
        DaemonAction::Start => {
            if client::daemon_running().await {
                return Ok(Output::with(
                    serde_json::json!({"running": true}),
                    "Already running.\n".into(),
                ));
            }
            client::start_daemon()?;
            let v = client::send(&Request::DaemonStatus).await?;
            Ok(Output::with(v, "Daemon started.\n".into()))
        }
        DaemonAction::Stop => {
            if !client::daemon_running().await {
                return Ok(Output::with(
                    serde_json::json!({"running": false}),
                    "Not running.\n".into(),
                ));
            }
            let v = client::send(&Request::DaemonShutdown).await?;
            Ok(Output::with(v, "Daemon stopping.\n".into()))
        }
        DaemonAction::Status => {
            if !client::daemon_running().await {
                return Ok(Output::plain(serde_json::json!({"running": false})));
            }
            let v = client::send(&Request::DaemonStatus).await?;
            Ok(Output::with(
                v.clone(),
                format!(
                    "Running since {}\n",
                    v["started_at"].as_str().unwrap_or("?")
                ),
            ))
        }
    }
}

/// `cairn update` — report, and install when asked.
async fn update_command(check_only: bool) -> Result<Output, WireError> {
    let outcome = if check_only {
        update::check().await?
    } else {
        update::apply().await?
    };

    let mut text = format!("Installed  {}\n", outcome.current);
    match &outcome.latest {
        Some(latest) => {
            text.push_str(&format!("Latest     {}\n", latest.version));
            if outcome.installed {
                text.push_str("\nUpdated. Restart the daemon so it runs the new build:\n");
                text.push_str("  cairn daemon stop && cairn daemon start\n");
            } else if outcome.update_available {
                text.push_str(&format!("\n{} is available.\n", latest.version));
                text.push_str("Run `cairn update` to install it, or read about it first:\n");
                text.push_str(&format!("  {}\n", latest.url));
            } else {
                text.push_str("\nAlready up to date.\n");
            }
        }
        None => text.push_str("Latest     could not be determined\n"),
    }

    let value = serde_json::json!({
        "current": outcome.current,
        "latest": outcome.latest,
        "update_available": outcome.update_available,
        "installed": outcome.installed,
        "installed_to": outcome
            .installed_to
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
    });
    Ok(Output::with(value, text))
}
