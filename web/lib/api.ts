/**
 * Typed client for the Cairn server API (contracts/server-api.md).
 *
 * The browser authenticates with the session cookie set at sign-in, so every
 * request is credentialed.
 */

export const API_BASE =
  process.env.NEXT_PUBLIC_CAIRN_API ?? "http://127.0.0.1:8080";

export class ApiError extends Error {
  constructor(
    public code: string,
    message: string,
    public status: number,
  ) {
    super(message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${API_BASE}${path}`, {
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

export interface User {
  id: string;
  email: string;
  display_name: string;
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
