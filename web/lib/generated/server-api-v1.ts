// Generated from crates/cairn-server/contracts/web-api-v1.ts. Do not edit.
export const API_CONTRACT_VERSION = "v1";

export interface LoginResponse { id: string; }
export interface OkResponse { ok: boolean; }
export interface TokensResponse { tokens: ApiToken[]; }
export interface RevokedResponse { revoked: string; }
export interface ProjectsResponse { projects: Project[]; }
export interface CreatedProject { id: string; name: string; }
export interface CreatedMemory { id: string; applied: "accepted" | "duplicate"; }
export interface CreatedKnowledge { id: string; owner_user_id: string; }
export interface TeamProposal { id: string; state: "proposed"; proposed_by_user_id: string; }
export interface PromotedPattern { pattern_id: string; owner_user_id: string; domain: "personal"; trust: "sanitized"; content_key: string; stored: boolean; }
export interface SessionsResponse { sessions: Session[]; }
export interface HandoffResponse { handoff: Handoff; }
export interface DeletedResponse { deleted: string; }
export interface MemoryDetailResponse { memory: MemoryDetail; }
export interface HealthRowsResponse { rows: HealthRow[]; }
export interface UsersResponse { users: Account[]; }
export interface ResetPasswordResponse { id: string; temporary_password: string; }
export interface ChangedPassword { changed: boolean; }
export interface ProjectMembersResponse { members: ProjectMember[]; }
export interface ProjectMember { user_id: string; email: string; display_name: string; added_by_user_id: string | null; created_at: string; }
export interface GraphResponse { seed: string; hops: number; max_hops: number; max_edges: number; truncated: boolean; edges: GraphEdge[]; }
export interface GraphEdge { from: string; to: string; kind: string; depth: number; }
export interface ReplayResponse { events: ReplayEvent[]; limit: number; read_only: true; content_available: false; }
export interface ReplayEvent { event_id: string; session_id: string; kind: string; accepted_at: string; }
export interface Analytics { capture: number; consolidation: number; retrieval: number; delivery: number; failures: number; latency: { count: number; average_ms: number }; derived_from_existing_records: true; }

export interface Release {
  tag: string;
  version: string;
  url: string;
}

export interface VersionInfo {
  current: string;
  latest: Release | null;
  update_available: boolean;
  /** Null when the lookup has never succeeded — not the same as up to date. */
  checked_at: string | null;
}

export interface User {
  id: string;
  email: string;
  display_name: string;
  /**
   * What this account may do server-wide, answered by the server on every
   * request rather than cached from sign-in.
   *
   * Used to decide which navigation entries are worth showing, and never to
   * decide what a page may render. Every admin-gated route refuses a member
   * itself; the navigation only avoids offering a door that will not open
   * (FR-892).
   */
  role: "admin" | "member";
  status: "active" | "disabled";
}

export interface ApiToken {
  id: string;
  name: string;
  created_at: string;
  last_used_at: string | null;
  revoked_at: string | null;
}

export interface CreatedToken {
  id: string;
  name: string;
  /** Shown once, then unrecoverable. */
  token: string;
}

export interface Project {
  id: string;
  name: string;
  repository_remote: string | null;
  created_at: string;
}

export interface ProjectOverview {
  project: Project;
  counts: {
    sessions: number;
    memories: number;
  };
  branches: { branch: string; sessions: number; last_seen: string | null }[];
  recent_sessions: Session[];
}

export interface Session {
  id: string;
  agent: string;
  branch: string;
  commit_sha?: string | null;
  status: "active" | "completed" | "interrupted";
  started_at: string;
  ended_at: string | null;
  end_reason?: string | null;
  has_handoff?: boolean;
}

export interface Handoff {
  id: string;
  session_id: string;
  trigger: "pre_compact" | "session_end" | "recovered";
  goal: string;
  progress: string;
  completed_work: string[];
  remaining_work: string[];
  changed_files: string[];
  decisions: string[];
  failures: string[];
  /**
   * The runner's name, with flags and paths already stripped.
   *
   * Named `runner`, not `command`: a handoff carries no command string
   * (FR-532). The recursive wire denylist screens *field names*, so a key
   * called `command` anywhere inside a handoff payload is refused on sight,
   * which would make every handoff carrying a completed test run undeliverable.
   */
  tests_executed: { runner: string; outcome: string }[];
  repository_state: {
    branch: string;
    commit_sha: string | null;
    staged: number;
    unstaged: number;
    untracked: number;
  };
  next_step: string;
  agent_note: string | null;
  /** Identifiers and a count. The observations stayed on the capturing machine. */
  evidence: { observation_ids: string[]; evidence_count: number };
  created_at: string;
}

export interface Memory {
  id: string;
  type: string;
  scope: string;
  scope_key: string;
  content: string;
  state: string;
  superseded_by_id: string | null;
  importance: string;
  pinned: boolean;
  /**
   * The list route's verification is a bare state string; the detail route
   * replaces the same key with an object (see `MemoryDetail`).
   *
   * Two shapes under one name, which is the server's shape and not a choice
   * made here — spelled out because a component written against one and handed
   * the other renders `[object Object]` rather than failing.
   */
  verification: string | null;
  verification_authority: string | null;
  /** Null for rows written before the column existed — not `explicit`. */
  origin_kind: string | null;
  reinforcement_count: number;
  relation_count: number;
  provenance: {
    session_id: string;
    observation_ids: string[];
    evidence_count: number;
  };
  created_at: string;
  updated_at: string;
}

export interface MemoryPage {
  memories: Memory[];
  /** How many arrived. Equal to `limit` when the page may have been truncated. */
  total: number;
  /** The bound the server actually applied, after clamping. */
  limit: number;
}

export interface MemorySearch {
  q?: string;
  scope?: string;
  scope_key?: string;
  type?: string;
  state?: string;
  limit?: number;
}
export interface CreateProjectBody { name: string; repository_remote?: string; }
export interface CreateMemoryBody { type: string; scope: string; content: string; scope_key?: string; topic_key?: string; value_key?: string; session_id?: string; command_id?: string; }
export interface CreatePersonalKnowledgeBody { type: string; content: string; topic_key?: string; value_key?: string; command_id?: string; }
export interface ProposeTeamKnowledgeBody { type: string; content: string; topic_key?: string; value_key?: string; command_id?: string; }
export interface PromotePatternBody { title: string; problem: string; root_cause: string; approach: string; constraints?: string[]; applicability?: string[]; command_id?: string; }

// ---------------------------------------------------------------------------
// The web control plane (contracts/web-control-plane.md)
// ---------------------------------------------------------------------------

/**
 * A reference to one knowledge record, in the discriminated form every
 * control-plane response uses.
 *
 * Both parts always travel and both parts must always be rendered. A knowledge
 * reference is a domain *and* an id: two domains can hold the same UUID, so an
 * id on its own names nothing and a link that drops the domain can open the
 * wrong record. A pattern reference has no domain at all — `domain` is `null`
 * there rather than `"personal"`, which is why this is a discriminated shape
 * and not a string plus an optional field.
 */
export interface Reference {
  ref_kind: "knowledge" | "pattern";
  domain: "project" | "personal" | "team" | null;
  knowledge_id: string;
  /** The canonical `knowledge:<domain>:<id>` or `pattern:<id>` key. */
  reference_key: string;
}

export interface PageQuery {
  cursor?: string;
  limit?: number;
}

/** One funnel stage. */
export interface FunnelStage {
  stage: string;
  /**
   * Zero and unavailable are different answers and this field keeps them apart.
   *
   * `0` is the query having run against the mechanism and found nothing. `null`
   * is the mechanism not existing on this deployment, so nothing can be said
   * either way. Any `?? 0` applied to this field turns "nobody looked" into
   * "nothing happened" and is a bug (FR-880).
   */
  count: number | null;
}

export interface Funnel {
  /** Null means the project's whole history rather than a window. */
  window_days: number | null;
  stages: FunnelStage[];
}

export interface ActivityQuery {
  /** Absent means the server's declared default subset (FR-882). */
  kinds?: string[];
  cursor?: string;
  limit?: number;
}

export interface ActivityItem {
  family: "safe_event" | "candidate_decision";
  id: string;
  at: string;
  kind: string;
  agent: string | null;
  session_id: string | null;
  /** The event's approved per-kind structure. Null for candidate decisions. */
  content: Record<string, unknown> | string | null;
  refusal_reason: string | null;
  /**
   * The record a decision produced, or null.
   *
   * Null covers two cases the view must not distinguish: the item is an
   * arrival and produced nothing, or it produced a record this reader may not
   * see. The server withholds the second rather than emitting a bare id,
   * because an id still discloses that the record exists (FR-846a).
   */
  reference: Reference | null;
}

export interface ActivityPage {
  items: ActivityItem[];
  /** Null at the end of the feed. */
  cursor: string | null;
  limit: number;
  /** The subset the server actually applied, declared rather than inferred. */
  kinds: string[];
}

export interface MemoryRelation {
  direction: "incoming" | "outgoing";
  kind: string;
  basis: string;
  decided_by_session: string;
  decided_at: string;
  /** The other end, always a complete reference. */
  other: Reference;
}

/**
 * What supports a memory, in counts.
 *
 * There is no field here that could carry evidence content, a file path or
 * command output, because the server has never held any of it — evidence stays
 * on the machine that captured it. `local_to_session` names where it is so the
 * view can say so instead of rendering an empty section (FR-893).
 */
export interface EvidenceSummary {
  observation_count: number;
  evidence_count: number;
  evidence_fact_count: number;
  /** Verifier *kinds* — the sort of check that ran, never its subject. */
  verifier_kinds: string[];
  content_available: false;
  local_to_session: string;
}

export interface MemoryVerification {
  state: string | null;
  authority: string | null;
  last_verified_at: string | null;
  /** A verification that has expired: true was established, but not now. */
  stale: boolean;
}

export interface RetrievalUsage {
  trace_id: string;
  session_id: string;
  trigger: string;
  delivery_point: string;
  delivery_state: string;
  status: "considered" | "selected";
  rank: number | null;
  at: string;
}

export interface MemoryDetail extends Omit<Memory, "verification"> {
  provenance: Memory["provenance"] & { evidence_content_available: false };
  evidence_summary: EvidenceSummary;
  verification: MemoryVerification;
  relations: MemoryRelation[];
  /** The twenty most recent. The rest are reachable from the traces list. */
  retrieval_usage: RetrievalUsage[];
}

export interface TraceQuery extends PageQuery {
  /** Canonical reference key; a key the reader may not see returns an empty page. */
  reference_key?: string;
  session_id?: string;
}

export interface TraceSummary {
  trace_id: string;
  session_id: string;
  trigger: string;
  delivery_point: string;
  degradation_level: string | null;
  delivery_state: "requested" | "generated" | "transmitted" | "failed";
  acknowledgement_state: "unavailable" | "acknowledged";
  failure_reason: string | null;
  created_at: string;
}

export interface TracePage {
  traces: TraceSummary[];
  cursor: string | null;
  limit: number;
}

export interface TraceItem extends Reference {
  status: "considered" | "selected";
  selection_rule: string | null;
  rank: number;
  source_updated_at: string;
}

/**
 * One retrieval, in full — and there is no briefing text in it.
 *
 * The assembled briefing is not a field that is withheld: the server never
 * stored one (FR-839). So there is nothing here for a view to render and
 * nothing for a view to reconstruct from `items`, which record what was
 * selected rather than what was written.
 */
export interface TraceDetail {
  trace_id: string;
  session_id: string;
  trigger: string;
  delivery_point: string;
  degradation_level: string | null;
  delivery_state: TraceSummary["delivery_state"];
  acknowledgement_state: TraceSummary["acknowledgement_state"];
  failure_reason: string | null;
  created_at: string;
  items: TraceItem[];
  /**
   * Present only when the signed-in account is the one that made this
   * retrieval. Absent — not null — for a co-member reading somebody else's
   * trace, which is why the view must test for the key rather than for a
   * falsy value.
   */
  budget?: { tokens: number | null; spent: number | null };
  latency_ms?: number | null;
}

export interface HealthRow {
  /**
   * Who reported it. Both halves of the attribution are needed, because
   * `writer_id` is a label the reporting client chooses and two accounts can
   * pick the same one — a shared CI name is the obvious case. Without this a
   * reader sees two contradictory cells for one machine and no way to tell
   * whose observation is whose (FR-857).
   */
  account_id: string;
  /** The machine. A capability verified on one machine is not verified everywhere. */
  writer_id: string;
  agent: string;
  capability: string;
  stage: string;
  status:
    | "supported"
    | "unsupported_by_vendor"
    | "declined_by_cairn"
    | "adapter_unimplemented"
    | "runtime_failure"
    | "no_evidence";
  /** Configuration read back, or behaviour observed. Never folded into `status`. */
  evidence_kind: "introspection" | "observation" | null;
  observed_at: string | null;
  degraded: boolean | null;
}

export interface ApplicabilityFact {
  kind: string;
  value: string;
}

export interface PersonalKnowledge {
  id: string;
  knowledge_type: string;
  content: string;
  topic_key: string | null;
  value_key: string | null;
  writer_id: string;
  writer_seq: number;
  created_at: string;
  superseded_by_id: string | null;
  forgotten_at: string | null;
  applicability: ApplicabilityFact[];
}

export interface PersonalKnowledgePage {
  items: PersonalKnowledge[];
  cursor: string | null;
  limit: number;
}

export interface TeamKnowledge {
  id: string;
  knowledge_type: string;
  content: string;
  topic_key: string | null;
  value_key: string | null;
  applicability: ApplicabilityFact[];
  state: "proposed" | "authoritative" | "retired";
  proposed_by_user_id: string;
  ratified_by_user_id: string | null;
  ratified_at: string | null;
  writer_id: string;
  writer_seq: number;
  created_at: string;
  superseded_by_id: string | null;
  retired_by_user_id: string | null;
  retired_at: string | null;
}

export interface TeamKnowledgePage {
  items: TeamKnowledge[];
  cursor: string | null;
  limit: number;
  /** Whose view this page reflects; a cursor from one caller is not another's. */
  visibility: string;
}

export interface TeamTransition {
  id: string;
  state: TeamKnowledge["state"];
  ratified_by_user_id?: string;
  ratified_at?: string;
  retired_by_user_id?: string;
  retired_at?: string;
  supersedes?: string | null;
}

export interface Pattern {
  pattern_id: string;
  title: string;
  problem: string;
  root_cause: string;
  approach: string;
  constraints: string[];
  applicability: ApplicabilityFact[];
  trust: string;
  content_key: string;
  created_at: string;
  updated_at: string;
}

export interface PatternList {
  /** How many the owner holds, which is not how many arrived. */
  total: number;
  returned: number;
  /** The bound applied, or null when none was asked for. */
  limit: number | null;
  /**
   * Which order this reply is in. A bounded read pages over `pattern_id`,
   * which is stable; an unbounded one is newest-first. Said rather than
   * assumed, so a paging caller cannot read one as the other.
   */
  order: "pattern_id" | "updated_at desc";
  /** Where to resume, or null when this page is the last. */
  cursor: string | null;
  patterns: Pattern[];
}

export interface ConsolidationRun {
  run_id: string;
  session_id: string | null;
  started_at: string;
  finished_at: string | null;
  state: string;
  events_claimed: number | null;
  candidates_proposed: number | null;
  candidates_accepted: number | null;
  candidates_refused: number | null;
  refusal_reasons: { reason: string; n: number }[];
  extractor_kind: string;
}

export interface ConsolidationRunPage {
  runs: ConsolidationRun[];
  cursor: string | null;
  limit: number;
}

export interface ConsolidationHealth {
  backlog_depth: number;
  /** Absent when there is no backlog — a different answer from zero. */
  oldest_enqueued_at: string | null;
  failed_events: number;
  runs_finished: number;
  runs_failed: number;
  candidates_proposed: number;
  candidates_accepted: number;
  candidates_refused: number;
}

/**
 * Deployment-wide health.
 *
 * Each section is `null` on a deployment whose schema predates the tables
 * behind it — the same distinction the funnel makes, one level up: a server
 * without the tables has not observed nothing, it has observed nothing yet
 * knowable (FR-880).
 */
export interface SystemHealth {
  ingest: {
    events_received: number;
    last_received_at: string | null;
    capture_failures: number;
    failures_by_disposition: { disposition: string; n: number }[];
  } | null;
  consolidation: ConsolidationHealth | null;
  retrieval: {
    traces: number;
    failed: number;
    /** Retrieval that never finished, and a briefing nobody confirmed arrived. */
    never_generated: number;
    never_transmitted: number;
    transmitted: number;
    last_trace_at: string | null;
  } | null;
}

export interface Account extends User {
  must_change_password: boolean;
  created_at: string;
}

/**
 * A freshly created account.
 *
 * Not an `Account`: the creation response carries no `created_at`, and it
 * carries one field no later read ever will. The temporary password exists on
 * this response and nowhere else — there is no route that reads it back, for
 * anyone, because a password that can be retrieved after creation is a password
 * stored in retrievable form.
 */
export interface CreatedAccount {
  id: string;
  email: string;
  display_name: string;
  role: User["role"];
  status: User["status"];
  must_change_password: boolean;
  temporary_password: string;
}
