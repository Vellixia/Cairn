/**
 * Typed client for the Cairn server API (contracts/server-api.md).
 *
 * The browser authenticates with the session cookie set at sign-in, so every
 * request is credentialed.
 */

declare global {
  interface Window {
    __CAIRN_API_ORIGIN__?: string;
  }
}

/**
 * Where the API lives, resolved per call rather than baked into the bundle.
 *
 * The published image used to carry `http://127.0.0.1:8080` because
 * `NEXT_PUBLIC_*` is inlined at `next build`, which meant every operator with
 * their own domain had to rebuild the image for that one domain. Resolution
 * order now:
 *
 * 1. `window.__CAIRN_API_ORIGIN__`, written per request by the root layout from
 *    the container's `CAIRN_API_ORIGIN`. The runtime knob: set it and restart,
 *    no rebuild.
 * 2. `NEXT_PUBLIC_CAIRN_API`, still inlined at build time, for anyone who
 *    prefers a baked origin — and how the e2e job points the UI at its server.
 * 3. In development only, the loopback server. `next dev` serves the UI on :3100
 *    while cairn-server runs natively on :8080, so development is the one case
 *    that is genuinely split-origin; defaulting it here keeps `npm run dev`
 *    working with no configuration and without a committed env file, which is
 *    gitignored anyway.
 * 4. Same origin. The empty base sends requests to `/api/...` on whichever host
 *    served the page, which is the recommended layout (web at `/`, API at
 *    `/api` behind one hostname) and what the published image now defaults to.
 *
 * A relative base is safe because every caller of `request` is a client
 * component: nothing below is fetched during server rendering, so no absolute
 * URL is ever required.
 */
export function apiBase(): string {
  const runtime =
    typeof window === "undefined" ? undefined : window.__CAIRN_API_ORIGIN__;
  const devDefault =
    process.env.NODE_ENV === "development" ? "http://127.0.0.1:8080" : "";
  const base =
    runtime?.trim() || process.env.NEXT_PUBLIC_CAIRN_API?.trim() || devDefault;
  // A trailing slash would produce `//api/...`, which is a protocol-relative URL.
  return base.replace(/\/+$/, "");
}

export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

/** Whether an error is the server saying "you are not signed in". */
export function isUnauthorized(error: unknown): boolean {
  return error instanceof ApiError && error.status === 401;
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${apiBase()}${path}`, {
    ...init,
    credentials: "include",
    headers: { "content-type": "application/json", ...(init?.headers ?? {}) },
  });
  const body = await response.json().catch(() => null);
  if (!response.ok) {
    throw new ApiError(
      body?.error?.code ?? "internal",
      body?.error?.message ?? response.statusText,
      response.status,
    );
  }
  return body as T;
}

/**
 * The query part of a URL, with absent parameters left out entirely.
 *
 * Absent and empty are different to these routes. `?kinds=` would be a `kinds`
 * parameter naming nothing, which the activity route refuses by name rather
 * than treating as "give me the default" — so a parameter with no value must
 * not be sent at all.
 */
function queryString(params: Record<string, string | number | undefined>): string {
  const q = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== "") q.set(key, String(value));
  }
  const suffix = q.toString();
  return suffix ? `?${suffix}` : "";
}

export const api = {
  /** Unauthenticated: the version of a service is not a secret. */
  version: () => request<VersionInfo>("/api/version"),
  me: () => request<User>("/api/auth/me"),
  login: (email: string, password: string) =>
    request<{ id: string }>("/api/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  logout: () => request<{ ok: boolean }>("/api/auth/logout", { method: "POST" }),

  /** Personal API tokens: the credential `cairnd` carries (D10). */
  tokens: () => request<{ tokens: ApiToken[] }>("/api/tokens"),
  /** The plaintext comes back exactly once and is never stored server-side. */
  createToken: (name: string) =>
    request<CreatedToken>("/api/tokens", {
      method: "POST",
      body: JSON.stringify({ name }),
    }),
  revokeToken: (id: string) =>
    request<{ revoked: string }>(`/api/tokens/${id}`, { method: "DELETE" }),

  projects: () => request<{ projects: Project[] }>("/api/projects"),
  project: (id: string) => request<ProjectOverview>(`/api/projects/${id}`),
  tasks: (id: string, status?: string) =>
    request<{ tasks: Task[] }>(
      `/api/projects/${id}/tasks${status ? `?status=${status}` : ""}`,
    ),
  sessions: (id: string) =>
    request<{ sessions: Session[] }>(`/api/projects/${id}/sessions`),
  handoff: (sessionId: string) =>
    request<{ handoff: Handoff }>(`/api/sessions/${sessionId}/handoff`),
  /**
   * The memory explorer's page.
   *
   * `limit` travels back on the response as well as out on the request, and the
   * explorer needs both: `total` is how many rows arrived, which is the same
   * number for a full page and for a project holding exactly that many
   * memories. Only the two together say whether the list was truncated
   * (FR-895).
   */
  memories: (id: string, params: MemorySearch) =>
    request<MemoryPage>(
      `/api/projects/${id}/memories${queryString({
        q: params.q,
        scope: params.scope,
        scope_key: params.scope_key,
        type: params.type,
        state: params.state,
        limit: params.limit,
      })}`,
    ),
  deleteMemory: (memoryId: string) =>
    request<{ deleted: string }>(`/api/memories/${memoryId}`, {
      method: "DELETE",
    }),
  syncStatus: (id: string) =>
    request<SyncStatus>(`/api/projects/${id}/sync-status`),

  // ---------------------------------------------------------------------
  // The web control plane (contracts/web-control-plane.md)
  //
  // Every project-scoped read below is membership-guarded by the server and
  // answers a non-member with `403`, never an empty list. Nothing here filters
  // on the client, and nothing here decides who may see what: the callers
  // render whatever the server returns, including its refusals (FR-892,
  // FR-894a).
  // ---------------------------------------------------------------------

  /** The twelve funnel stages. `days` absent means the project's whole history. */
  funnel: (id: string, days?: number) =>
    request<Funnel>(
      `/api/projects/${id}/funnel${days ? `?days=${days}` : ""}`,
    ),
  activity: (id: string, params: ActivityQuery = {}) =>
    request<ActivityPage>(
      `/api/projects/${id}/activity${queryString({
        // Joined here rather than repeated per caller: the server takes one
        // comma-separated parameter and refuses a name it does not recognise.
        kinds: params.kinds?.join(","),
        cursor: params.cursor,
        limit: params.limit,
      })}`,
    ),
  consolidationRuns: (id: string, params: PageQuery = {}) =>
    request<ConsolidationRunPage>(
      `/api/projects/${id}/consolidation-runs${queryString({ ...params })}`,
    ),
  memory: (memoryId: string) =>
    request<{ memory: MemoryDetail }>(`/api/memories/${memoryId}`),
  retrievalTraces: (id: string, params: TraceQuery = {}) =>
    request<TracePage>(
      `/api/projects/${id}/retrieval-traces${queryString({ ...params })}`,
    ),
  retrievalTrace: (traceId: string) =>
    request<TraceDetail>(`/api/retrieval-traces/${traceId}`),
  integrationHealth: (id: string) =>
    request<{ rows: HealthRow[] }>(`/api/projects/${id}/integration-health`),

  /**
   * The caller's own personal knowledge, and only ever the caller's.
   *
   * There is no owner parameter here because the route has none — the owner is
   * the session cookie. A signature that could name somebody else is the thing
   * the server deliberately does not offer (FR-708d).
   */
  personalKnowledge: (params: PageQuery = {}) =>
    request<PersonalKnowledgePage>(
      `/api/personal/knowledge${queryString({ ...params })}`,
    ),
  /**
   * Owner-scoped for the same reason, and bounded only when asked.
   *
   * The bound is opt-in on this route alone, because the daemon's pattern cache
   * refills from it: a default page would silently truncate that cache. A screen
   * asks for a page and compares `returned` against `total` to know whether it
   * saw everything; omitting `limit` still returns the lot.
   */
  patterns: (limit?: number) =>
    request<PatternList>(`/api/patterns${queryString({ limit })}`),
  teamKnowledge: (params: PageQuery = {}) =>
    request<TeamKnowledgePage>(`/api/team/knowledge${queryString({ ...params })}`),

  /**
   * Ratify and retire: the two pre-existing atomic transitions, called as they
   * are.
   *
   * Each server handler is a single compare-and-swap — `WHERE state =
   * 'proposed'` and `WHERE state = 'authoritative'` — so two administrators
   * acting at once race inside PostgreSQL and exactly one wins. A client that
   * read the state, decided, and then posted an unconditional change would put
   * that race back in the browser, where it cannot be resolved (FR-889a). So
   * these send the id and nothing about the state they expect to find.
   */
  /**
   * Ratify a proposal. Admin-only, and refused server-side for anyone else.
   *
   * **The empty object is not decoration.** `request` sets
   * `content-type: application/json` on every call, and a request that declares
   * a JSON body and sends none is a request the server's extractor rejects —
   * these two routes take an optional body, and "absent" means no content-type,
   * not a content-type with nothing behind it. Sending `{}` says what the header
   * already claimed. Without it both transitions answered 400 and the row stayed
   * `proposed` while the UI reported nothing wrong.
   */
  ratifyTeam: (id: string) =>
    request<TeamTransition>(`/api/team/${id}/ratify`, {
      method: "POST",
      body: JSON.stringify({}),
    }),
  retireTeam: (id: string) =>
    request<TeamTransition>(`/api/team/${id}/retire`, {
      method: "POST",
      body: JSON.stringify({}),
    }),

  /** Admin-only on the server: a member is refused before the handler runs. */
  systemHealth: () => request<SystemHealth>("/api/system/health"),
  /** Any authenticated caller — the backlog is a deployment fact, not a project's. */
  consolidationHealth: () =>
    request<ConsolidationHealth>("/api/consolidation/health"),

  adminUsers: () => request<{ users: Account[] }>("/api/admin/users"),
  createAdminUser: (email: string, displayName: string) =>
    request<CreatedAccount>("/api/admin/users", {
      method: "POST",
      body: JSON.stringify({ email, display_name: displayName }),
    }),
  patchAdminUser: (id: string, patch: { role?: string; status?: string }) =>
    request<Account>(`/api/admin/users/${id}`, {
      method: "PATCH",
      body: JSON.stringify(patch),
    }),
  resetAdminUserPassword: (id: string) =>
    request<{ id: string; temporary_password: string }>(
      `/api/admin/users/${id}/reset-password`,
      { method: "POST" },
    ),
};

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
    tasks: number;
    open_tasks: number;
    sessions: number;
    memories: number;
  };
  branches: { branch: string; sessions: number; last_seen: string | null }[];
  recent_sessions: Session[];
}

export interface Task {
  id: string;
  title: string;
  goal: string;
  acceptance_criteria: string[];
  status: "todo" | "in_progress" | "done" | "blocked";
  updated_at: string;
}

export interface Session {
  id: string;
  task_id: string | null;
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

export interface SyncStatus {
  applied_items: number;
  last_applied_at: string | null;
}

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
