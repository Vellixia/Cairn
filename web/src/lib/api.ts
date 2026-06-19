// Typed HTTP client for the Cairn API.
//
// The dashboard is a static export served BY the cairn server, so by default it talks to
// whatever origin it was loaded from (`window.location.origin`). That means opening the
// dashboard at http://your-server:7777 just works — no rebuild, no hardcoded localhost.
//
// All calls send `credentials: "include"` so the cairn_session cookie rides along. On a 401
// from any non-auth endpoint, the user is bounced to /login (or /setup on first run).

export function resolveApiBase(): string {
  if (typeof process !== "undefined" && process.env.NEXT_PUBLIC_CAIRN_API) {
    return process.env.NEXT_PUBLIC_CAIRN_API;
  }
  if (typeof window !== "undefined") {
    return window.location.origin;
  }
  return "http://127.0.0.1:7777";
}

export const API_BASE = resolveApiBase();

export class ApiError extends Error {
  readonly status: number;
  readonly body: unknown;
  constructor(status: number, message: string, body: unknown) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
  }
}

function isAuthPath(path: string): boolean {
  return (
    path === "/api/auth/login" ||
    path === "/api/auth/logout" ||
    path === "/api/auth/setup" ||
    path === "/api/auth/status" ||
    path === "/api/auth/me" ||
    path === "/api/health" ||
    path === "/api/pair/claim"
  );
}

export async function request<T>(
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    credentials: "include",
    ...init,
    headers: {
      "content-type": "application/json",
      ...(init.headers ?? {}),
    },
  });
  if (!res.ok) {
    let body: unknown = null;
    try {
      body = await res.json();
    } catch {
      try { body = await res.text(); } catch { /* ignore */ }
    }
    const message =
      typeof body === "object" && body && "error" in body
        ? String((body as { error: unknown }).error)
        : `${res.status} ${res.statusText}`;
    // Single bounce rule: any 401 from a non-auth path sends the user to /login.
    if (res.status === 401 && !isAuthPath(path) && typeof window !== "undefined") {
      const from = encodeURIComponent(window.location.pathname + window.location.search);
      window.location.assign(`/login?from=${from}`);
    }
    throw new ApiError(res.status, message, body);
  }
  // 204 / empty body
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export function getJSON<T>(path: string): Promise<T> {
  return request<T>(path, { method: "GET" });
}

export function postJSON<T>(path: string, body: unknown): Promise<T> {
  return request<T>(path, { method: "POST", body: JSON.stringify(body) });
}

export function delJSON<T>(path: string): Promise<T> {
  return request<T>(path, { method: "DELETE" });
}

// ---- Wire types -------------------------------------------------------------------------------

export interface AuthStatus {
  admin_exists: boolean;
  setup_required: boolean;
}

export interface Me {
  username: string;
  generation: number;
  login_at: number;
  expires_at: number;
}

export interface Health {
  status: string;
  name: string;
  version: string;
}

export interface Stats {
  memories: number;
  checkpoints?: number;
  preferences?: number;
  anchor?: string | null;
  reliability?: Reliability;
}

export interface Reliability {
  score: number;
  samples: number;
  ok: number;
  warn: number;
  danger: number;
  rollbacks: number;
}

export interface Checkpoint {
  id: string;
  created_at: string;
  files: number;
  label: string;
}
export interface RollbackReport {
  checkpoint_id: string;
  restored: string[];
  skipped: string[];
}

export type Sensitivity = "shareable" | "needs_review" | "private";
export interface Finding { kind: string; start: number; end: number; }
export interface Sanitized { text: string; findings: Finding[]; sensitivity: Sensitivity; }

export interface ShareExport {
  schema: string;
  version: number;
  total: number;
  shared: number;
  needs_review: number;
  withheld: number;
  memories: unknown[];
}
export interface PoolMemory {
  kind: string;
  content: string;
  concepts: string[];
  sensitivity: Sensitivity;
  redactions: number;
}
export interface Pool {
  schema: string;
  version: number;
  count: number;
  memories: PoolMemory[];
}

export interface Memory {
  id: string;
  kind: string;
  tier: string;
  content: string;
  concepts: string[];
  files: string[];
  importance: number;
  access_count: number;
  created_at: string;
  updated_at: string;
}
export interface ScoredMemory { memory: Memory; score: number; }

export interface ReadResult {
  path: string;
  hash: string;
  handle: string;
  status: "full" | "cached" | "diff" | "outline";
  lines: number;
  bytes: number;
  view: string;
  note: string;
  est_tokens: number;
}

export interface DeviceTokenMeta {
  id: string;
  name: string;
  scope: string;
  created_at: string;
  expires_at: string | null;
  last_used_at: string | null;
}

export interface IssuedToken extends DeviceTokenMeta {
  token: string;
}

export interface PairCode {
  code: string;
  name: string;
  expires_at: string;
}

export interface AuditEvent {
  ts: number;
  kind: string;
  actor: string;
  detail: string;
}
