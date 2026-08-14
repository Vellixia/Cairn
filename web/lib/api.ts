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

export const api = {
  /** Unauthenticated: the version of a service is not a secret. */
  version: () => request<VersionInfo>("/api/version"),
  me: () => request<User>("/api/auth/me"),
  login: (email: string, password: string) =>
    request<{ id: string }>("/api/auth/login", {
      method: "POST",
      body: JSON.stringify({ email, password }),
    }),
  register: (email: string, displayName: string, password: string) =>
    request<{ id: string }>("/api/auth/register", {
      method: "POST",
      body: JSON.stringify({ email, display_name: displayName, password }),
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
  memories: (id: string, params: MemorySearch) => {
    const q = new URLSearchParams();
    if (params.q) q.set("q", params.q);
    if (params.scope) q.set("scope", params.scope);
    if (params.type) q.set("type", params.type);
    if (params.state) q.set("state", params.state);
    const suffix = q.toString();
    return request<{ memories: Memory[]; total: number }>(
      `/api/projects/${id}/memories${suffix ? `?${suffix}` : ""}`,
    );
  },
  deleteMemory: (memoryId: string) =>
    request<{ deleted: string }>(`/api/memories/${memoryId}`, {
      method: "DELETE",
    }),
  syncStatus: (id: string) =>
    request<SyncStatus>(`/api/projects/${id}/sync-status`),
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
  tests_executed: { command: string; outcome: string }[];
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
  provenance: {
    session_id: string;
    observation_ids: string[];
    evidence_count: number;
  };
  created_at: string;
  updated_at: string;
}

export interface MemorySearch {
  q?: string;
  scope?: string;
  type?: string;
  state?: string;
}

export interface SyncStatus {
  applied_items: number;
  last_applied_at: string | null;
}
