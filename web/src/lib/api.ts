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

export interface Memory {
  id: string;
  kind: string;
  tier: string;
  /** Short scannable label. `null` for memories written before this field existed or by
   * callers that don't set one - the browser falls back to the first line of content. */
  title: string | null;
  content: string;
  /** Why this memory matters, kept separate from content. `null` when not provided. */
  reasoning: string | null;
  concepts: string[];
  files: string[];
  /** Provenance: the agent session that wrote this memory, if any. */
  session_id: string | null;
  /** Multi-tenant org tag; the implicit default org on self-hosted installs. */
  org_id: string;
  /** Flagged by the content scanner (possible secret/injection); surfaced, never hidden. */
  suspicious: boolean;
  importance: number;
  access_count: number;
  /** Confidence score [0.0, 1.0], evolves with reinforcement. Defaults to 0.5. */
  confidence: number;
  /** Pinned memories always surface first in wakeup regardless of score. */
  pinned: boolean;
  /** Edges: ids of memories this one was derived from. */
  derived_from: string[];
  /** Edges: ids of memories this one contradicts. */
  contradicts: string[];
  /** Edges: ids of memories this one supersedes. */
  supersedes: string[];
  /** Edges: file paths / symbols / project ids this memory applies to. */
  applies_to: string[];
  /** v0.8.0 Sprint 2: Global / Project / Session isolation scope. */
  scope_type: "global" | "project" | "session";
  scope_id: string | null;
  /** v0.8.0 Sprint 5: promotion-worthiness score in [0.0, 1.0]; 0.70-0.90 is the review band. */
  promo_score: number;
  /** v0.8.0 Sprint 5: once true, excluded from promotion scoring/suggestions. */
  promo_locked: boolean;
  created_at: string;
  updated_at: string;
}
/** Web redesign v2: filters for the Memory Browser's `GET /api/memory` list endpoint. */
export interface MemoryListFilters {
  scope_type?: string;
  scope_id?: string;
  tier?: string;
  kind?: string;
  pinned?: boolean;
  suspicious?: boolean;
  q?: string;
  sort?: "updated_at" | "created_at" | "importance" | "promo_score" | "access_count";
  limit?: number;
  offset?: number;
}

export interface MemoryListResponse {
  items: Memory[];
  total: number;
}

/** One guard drift event from `GET /api/guard/drift` (read-only; autopilot decides). */
export interface DriftEvent {
  id: number;
  ts: string;
  path: string;
  risk: "ok" | "warn" | "danger" | string;
  kind: string;
  detail: string;
  status: "pending" | "approved" | "rejected" | string;
}

/** Web redesign v2: one effective-config entry from `GET /api/config` (read-only). */
export interface ConfigEntry {
  key: string;
  value: unknown;
  env: string;
  group: string;
  description: string;
}

/** v0.8.0 Sprint 4/5: one entry in `GET /api/cron/history`. */
export interface CronRun {
  job: string;
  started_at: string;
  duration_ms: number;
  outcome: "ok" | "err";
  detail: string;
}

/** v0.8.0 Sprint 6: an ingested document, from `GET /api/documents`. */
export interface DocumentSummary {
  id: string;
  source: string;
  title: string;
  chunk_count: number;
  /** Web redesign v2 follow-up: `null` = global (visible everywhere); a project id = attached
   * to that project's own Documents section, plus the unfiltered global list. */
  project_id: string | null;
  updated_at: string;
}

/** v0.8.0 Sprint 6: one chunk hit from `GET /api/documents/search`. */
export interface DocumentChunkRecord {
  id: string;
  source: string;
  title: string;
  chunk_index: number;
  content: string;
  project_id: string | null;
  created_at: string;
}

/** v0.8.0 Sprint 3/10: a registered project, enriched with memory stats. */
export interface ProjectWithStats {
  id: string;
  name: string;
  path: string;
  first_seen: string;
  last_active: string;
  memory_count: number;
  last_memory_at: string | null;
}

/** v0.8.0 Sprint 4: one background job's schedule + last run, from `GET /api/cron/jobs`. */
export interface CronJobStatus {
  name: string;
  schedule: string;
  last_run: CronRun | null;
}

/** v0.8.0 Sprint 8: one promotion/demotion event from `GET /api/memory/promotion-log`. */
export interface PromotionLogEntry {
  id: string;
  memory_id: string;
  action: "promote" | "demote";
  old_scope_type: string;
  old_scope_id: string | null;
  score: number;
  reason: string;
  ts: string;
}

/** v0.8.0 Sprint 8: "while you were away" autopilot summary from `GET /api/memory/autopilot-digest`. */
export interface AutopilotDigest {
  promoted: number;
  demoted: number;
  drift_auto_approved: number;
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

export interface ArchitectureGodNode {
  name: string;
  edge_count: number;
  kind: string;
}

export interface ArchitectureBridge {
  name: string;
  centrality: number;
  kind: string;
}

export interface ArchitectureReport {
  project: string;
  file_count: number;
  edge_count: number;
  community_count: number;
  god_nodes: ArchitectureGodNode[];
  bridges: ArchitectureBridge[];
  cycles: string[][];
  isolation_ratio: number;
  markdown: string;
  language_breakdown: Record<string, number>;
  surprising_connections: string[];
}

