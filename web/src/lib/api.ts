// Typed HTTP client for the Cairn API.
//
// The dashboard is a static export served BY the cairn server, so by default it talks to
// whatever origin it was loaded from (`window.location.origin`). That means opening the
// dashboard at http://your-server:7777 just works --- no rebuild, no hardcoded localhost.
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

const AUTH_PATHS = new Set([
  "/api/auth/login",
  "/api/auth/logout",
  "/api/auth/setup",
  "/api/auth/status",
  "/api/auth/me",
  "/api/health",
  "/api/pair/claim",
]);

function isAuthPath(path: string): boolean {
  return AUTH_PATHS.has(path);
}

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

export interface RequestOptions extends Omit<RequestInit, "body"> {
  body?: unknown;
}

export async function request<T>(
  path: string,
  init: RequestOptions = {},
): Promise<T> {
  const { body, headers, ...rest } = init;
  const res = await fetch(`${API_BASE}${path}`, {
    credentials: "include",
    ...rest,
    headers: {
      "content-type": "application/json",
      ...(headers ?? {}),
    },
    body: body == null ? undefined : JSON.stringify(body),
  });
  if (!res.ok) {
    let parsed: unknown = null;
    try {
      parsed = await res.json();
    } catch {
      try {
        parsed = await res.text();
      } catch {
        /* ignore */
      }
    }
    const message =
      typeof parsed === "object" && parsed && "error" in parsed
        ? String((parsed as { error: unknown }).error)
        : `${res.status} ${res.statusText}`;
    if (res.status === 401 && !isAuthPath(path) && typeof window !== "undefined") {
      const from = encodeURIComponent(
        window.location.pathname + window.location.search,
      );
      window.location.assign(`/login?from=${from}`);
    }
    throw new ApiError(res.status, message, parsed);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export function getJSON<T>(path: string): Promise<T> {
  return request<T>(path, { method: "GET" });
}

export function postJSON<T>(path: string, body: unknown): Promise<T> {
  return request<T>(path, { method: "POST", body });
}

/// Like [`postJSON`] but sends a raw `ArrayBuffer` with a caller-supplied content type.
/// Used by the pack registry's `POST /registry/packs` endpoint, where the body is the
/// tarball bytes rather than a JSON document.
export async function postBinary<T>(
  path: string,
  body: ArrayBuffer,
  contentType: string,
): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    method: "POST",
    credentials: "include",
    headers: { "content-type": contentType },
    body,
  });
  if (!res.ok) {
    let parsed: unknown = null;
    try {
      parsed = await res.json();
    } catch {
      /* ignore */
    }
    const message =
      typeof parsed === "object" && parsed && "error" in parsed
        ? String((parsed as { error: unknown }).error)
        : `${res.status} ${res.statusText}`;
    throw new ApiError(res.status, message, parsed);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

export function delJSON<T>(path: string): Promise<T> {
  return request<T>(path, { method: "DELETE" });
}

// ---- Wire types -------------------------------------------------------------

export interface Me {
  username: string;
  generation: number;
  login_at: number;
  expires_at: number;
}

export interface AuthStatus {
  admin_exists: boolean;
  setup_required: boolean;
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
export interface Finding {
  kind: string;
  start: number;
  end: number;
}
export interface Sanitized {
  text: string;
  findings: Finding[];
  sensitivity: Sensitivity;
}

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
  /** Confidence score [0.0, 1.0], evolves with reinforcement. Defaults to 0.5. */
  confidence: number;
  /** Pinned memories always surface first in wakeup regardless of score. */
  pinned: boolean;
  created_at: string;
  updated_at: string;
}
export interface ScoredMemory {
  memory: Memory;
  score: number;
}

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

export interface LedgerEntry {
  id: number;
  ts: string;
  source: string;
  bytes_in: number;
  bytes_out: number;
  tokens_saved: number;
  cost_usd_saved: number;
  signature: string;
}
