/**
 * Typed client for the Cairn server API (contracts/server-api.md).
 *
 * The browser authenticates with the session cookie set at sign-in, so every
 * request is credentialed.
 */

import type * as Contract from "./generated/server-api-v1";
export type * from "./generated/server-api-v1";

type VersionInfo = Contract.VersionInfo;
type User = Contract.User;
type CreatedToken = Contract.CreatedToken;
type ProjectOverview = Contract.ProjectOverview;
type MemoryPage = Contract.MemoryPage;
type MemorySearch = Contract.MemorySearch;
type ActivityQuery = Contract.ActivityQuery;
type ActivityPage = Contract.ActivityPage;
type Funnel = Contract.Funnel;
type TraceQuery = Contract.TraceQuery;
type TracePage = Contract.TracePage;
type TraceDetail = Contract.TraceDetail;
type PageQuery = Contract.PageQuery;
type PersonalKnowledgePage = Contract.PersonalKnowledgePage;
type PatternList = Contract.PatternList;
type TeamKnowledgePage = Contract.TeamKnowledgePage;
type TeamTransition = Contract.TeamTransition;
type ConsolidationRunPage = Contract.ConsolidationRunPage;
type ConsolidationHealth = Contract.ConsolidationHealth;
type SystemHealth = Contract.SystemHealth;
type Account = Contract.Account;
type CreatedAccount = Contract.CreatedAccount;
type LoginResponse = Contract.LoginResponse;
type OkResponse = Contract.OkResponse;
type TokensResponse = Contract.TokensResponse;
type RevokedResponse = Contract.RevokedResponse;
type ProjectsResponse = Contract.ProjectsResponse;
type CreatedProject = Contract.CreatedProject;
type CreatedMemory = Contract.CreatedMemory;
type CreatedKnowledge = Contract.CreatedKnowledge;
type TeamProposal = Contract.TeamProposal;
type PromotedPattern = Contract.PromotedPattern;
type CreateProjectBody = Contract.CreateProjectBody;
type CreateMemoryBody = Contract.CreateMemoryBody;
type CreatePersonalKnowledgeBody = Contract.CreatePersonalKnowledgeBody;
type ProposeTeamKnowledgeBody = Contract.ProposeTeamKnowledgeBody;
type PromotePatternBody = Contract.PromotePatternBody;
type SessionsResponse = Contract.SessionsResponse;
type HandoffResponse = Contract.HandoffResponse;
type DeletedResponse = Contract.DeletedResponse;
type MemoryDetailResponse = Contract.MemoryDetailResponse;
type HealthRowsResponse = Contract.HealthRowsResponse;
type UsersResponse = Contract.UsersResponse;
type ResetPasswordResponse = Contract.ResetPasswordResponse;
type ChangedPassword = Contract.ChangedPassword;
type ProjectMembersResponse = Contract.ProjectMembersResponse;
type GraphResponse = Contract.GraphResponse;
type ReplayResponse = Contract.ReplayResponse;
type Analytics = Contract.Analytics;
type MemoryMutation = Contract.MemoryMutation;
type RelationMutation = Contract.RelationMutation;

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
    request<LoginResponse>("/api/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  logout: () => request<OkResponse>("/api/auth/logout", { method: "POST" }),
  changePassword: (newPassword: string) => request<ChangedPassword>("/api/auth/password", { method: "POST", body: JSON.stringify({ new_password: newPassword }) }),

  /** Personal API tokens: the credential `cairnd` carries (D10). */
  tokens: () => request<TokensResponse>("/api/tokens"),
  /** The plaintext comes back exactly once and is never stored server-side. */
  createToken: (name: string) =>
    request<CreatedToken>("/api/tokens", {
      method: "POST",
      body: JSON.stringify({ name }),
    }),
  revokeToken: (id: string) =>
    request<RevokedResponse>(`/api/tokens/${id}`, { method: "DELETE" }),

  projects: () => request<ProjectsResponse>("/api/projects"),
  createProject: (body: CreateProjectBody) =>
    request<CreatedProject>("/api/projects", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  project: (id: string) => request<ProjectOverview>(`/api/projects/${id}`),
  members: (id: string) => request<ProjectMembersResponse>(`/api/projects/${id}/members`),
  addMember: (id: string, userId: string) => request(`/api/projects/${id}/members`, { method: "POST", body: JSON.stringify({ user_id: userId }) }),
  removeMember: (id: string, userId: string) => request(`/api/projects/${id}/members`, { method: "DELETE", body: JSON.stringify({ user_id: userId }) }),
  sessions: (id: string) =>
    request<SessionsResponse>(`/api/projects/${id}/sessions`),
  handoff: (sessionId: string) =>
    request<HandoffResponse>(`/api/sessions/${sessionId}/handoff`),
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
  createMemory: (id: string, body: CreateMemoryBody) =>
    request<CreatedMemory>(`/api/projects/${id}/memories`, {
      method: "POST",
      body: JSON.stringify(body),
    }),
  deleteMemory: (memoryId: string) =>
    request<DeletedResponse>(`/api/memories/${memoryId}`, {
      method: "DELETE",
    }),
  graph: (projectId: string, memoryId: string, hops = 1) => request<GraphResponse>(`/api/projects/${projectId}/graph${queryString({ memory_id: memoryId, hops })}`),
  replay: (projectId: string) => request<ReplayResponse>(`/api/projects/${projectId}/replay`),
  analytics: (projectId: string) => request<Analytics>(`/api/projects/${projectId}/analytics`),
  reinforceMemory: (id: string) => request<MemoryMutation>(`/api/memories/${id}/reinforce`, { method: "POST", body: JSON.stringify({}) }),
  pinMemory: (id: string, pinned: boolean) => request<MemoryMutation>(`/api/memories/${id}/pin`, { method: "POST", body: JSON.stringify({ pinned }) }),
  forgetMemory: (id: string) => request<MemoryMutation>(`/api/memories/${id}/forget`, { method: "POST", body: JSON.stringify({}) }),
  supersedeMemory: (id: string, content: string) => request<MemoryMutation>(`/api/memories/${id}/supersede`, { method: "POST", body: JSON.stringify({ type: "fact", scope: "project", content }) }),
  relateMemory: (projectId: string, from: string, to: string, kind: string) => request<RelationMutation>(`/api/projects/${projectId}/memory-relations`, { method: "POST", body: JSON.stringify({ from_memory_id: from, to_memory_id: to, kind }) }),
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
    request<MemoryDetailResponse>(`/api/memories/${memoryId}`),
  retrievalTraces: (id: string, params: TraceQuery = {}) =>
    request<TracePage>(
      `/api/projects/${id}/retrieval-traces${queryString({ ...params })}`,
    ),
  retrievalTrace: (traceId: string) =>
    request<TraceDetail>(`/api/retrieval-traces/${traceId}`),
  integrationHealth: (id: string) =>
    request<HealthRowsResponse>(`/api/projects/${id}/integration-health`),

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
  createPersonalKnowledge: (body: CreatePersonalKnowledgeBody) =>
    request<CreatedKnowledge>("/api/personal/knowledge", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  /**
   * Owner-scoped for the same reason, and bounded only when asked.
   *
   * The bound is opt-in on this route alone, because the daemon's pattern cache
   * refills from it: a default page would silently truncate that cache. A screen
   * asks for a page and compares `returned` against `total` to know whether it
   * saw everything; omitting `limit` still returns the lot.
   */
  patterns: (params: PageQuery = {}) =>
    request<PatternList>(`/api/patterns${queryString({ ...params })}`),
  promotePattern: (body: PromotePatternBody) =>
    request<PromotedPattern>("/api/patterns", {
      method: "POST",
      body: JSON.stringify(body),
    }),
  teamKnowledge: (params: PageQuery = {}) =>
    request<TeamKnowledgePage>(`/api/team/knowledge${queryString({ ...params })}`),
  proposeTeamKnowledge: (body: ProposeTeamKnowledgeBody) =>
    request<TeamProposal>("/api/team/knowledge", {
      method: "POST",
      body: JSON.stringify(body),
    }),

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

  adminUsers: () => request<UsersResponse>("/api/admin/users"),
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
    request<ResetPasswordResponse>(
      `/api/admin/users/${id}/reset-password`,
      { method: "POST" },
    ),
};
