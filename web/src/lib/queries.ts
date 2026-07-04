import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  ApiError,
  delJSON,
  getJSON,
  postJSON,
  type ArchitectureReport,
  type AuditEvent,
  type AutopilotDigest,
  type ConfigEntry,
  type CronJobStatus,
  type CronRun,
  type DeviceTokenMeta,
  type DriftEvent,
  type Health,
  type IssuedToken,
  type LedgerEntry,
  type Me,
  type Memory,
  type MemoryListFilters,
  type MemoryListResponse,
  type PairCode,
  type DocumentSummary,
  type DocumentChunkRecord,
  type PromotionLogEntry,
  type ProjectWithStats,
  type Stats,
} from "@/lib/api";
import { useMeStore } from "@/lib/stores/me";
import { pollWhenOffline } from "@/lib/stores/events";
import type { IssueTokenInput, PairCodeInput } from "@/lib/forms/schemas";

// ---- query keys (single source of truth) ------------------------------------

export const qk = {
  health: ["health"] as const,
  me: ["auth", "me"] as const,
  stats: ["stats"] as const,
  anchor: ["guard", "anchor"] as const,
  memories: (limit: number) => ["memory", "wakeup", limit] as const,
  memoryList: (filters: MemoryListFilters) => ["memory", "list", filters] as const,
  memoryDetail: (id: string) => ["memory", "detail", id] as const,
  devicesTokens: ["devices", "tokens"] as const,
  devicesAudit: ["devices", "audit"] as const,
  ledger: (limit: number) => ["ledger", limit] as const,
  heatmap: (days: number) => ["memory", "heatmap", days] as const,
  architectureReport: ["memory", "architecture-report"] as const,
  promotionCandidates: ["memory", "promotion-candidates"] as const,
  config: ["config"] as const,
  cronJobs: ["cron", "jobs"] as const,
  cronHistory: (job?: string) => ["cron", "history", job ?? "all"] as const,
  promotionLog: (limit: number) => ["memory", "promotion-log", limit] as const,
  autopilotDigest: (hours: number) => ["automation", "digest", hours] as const,
  projects: ["projects"] as const,
  project: (id: string) => ["projects", id] as const,
  memoryByScope: (scopeType: string, scopeId: string, limit: number) =>
    ["memory", "by-scope", scopeType, scopeId, limit] as const,
  documents: ["documents"] as const,
  documentSearch: (q: string) => ["documents", "search", q] as const,
  drift: ["guard", "drift"] as const,
  graph: ["memory", "graph"] as const,
  activityAudit: ["activity", "audit"] as const,
  activityStats: ["activity", "stats"] as const,
  dashboardMetrics: ["dashboard", "metrics"] as const,
};

// ---- queries ----------------------------------------------------------------

export function useHealthQuery() {
  return useQuery({
    queryKey: qk.health,
    queryFn: () => getJSON<Health>("/api/health"),
    refetchInterval: 15_000,
  });
}

export function useMeQuery(enabled = true) {
  return useQuery({
    queryKey: qk.me,
    queryFn: () => getJSON<Me>("/api/auth/me"),
    enabled,
    retry: false,
  });
}

export function useStatsQuery() {
  return useQuery({
    queryKey: qk.stats,
    queryFn: () => getJSON<Stats>("/api/stats"),
    refetchInterval: pollWhenOffline(30_000),
  });
}

export function useAnchorQuery() {
  return useQuery({
    queryKey: qk.anchor,
    queryFn: () => getJSON<{ anchor: string | null }>("/api/guard/anchor"),
  });
}

export function useWakeupQuery(limit = 5) {
  return useQuery({
    queryKey: qk.memories(limit),
    queryFn: () => getJSON<Memory[]>(`/api/memory/wakeup?limit=${limit}`),
    refetchInterval: pollWhenOffline(60_000),
  });
}

// Web redesign v2: the Memory Browser's list query. Filter object must keep a consistent
// shape (undefined for absent keys) so react-query's key hashing doesn't churn.
export function useMemoryListQuery(filters: MemoryListFilters) {
  return useQuery({
    queryKey: qk.memoryList(filters),
    queryFn: () => {
      const params = new URLSearchParams();
      for (const [k, v] of Object.entries(filters)) {
        if (v !== undefined && v !== null && `${v}`.length > 0) params.set(k, `${v}`);
      }
      const qs = params.toString();
      return getJSON<MemoryListResponse>(`/api/memory${qs ? `?${qs}` : ""}`);
    },
    placeholderData: keepPreviousData,
    refetchInterval: pollWhenOffline(60_000),
  });
}

// Full single-memory record for the detail drawer (and edge-hopping).
export function useMemoryQuery(id: string | null) {
  return useQuery({
    queryKey: qk.memoryDetail(id ?? ""),
    queryFn: () => getJSON<Memory>(`/api/memory/${encodeURIComponent(id ?? "")}`),
    enabled: !!id,
  });
}

// Read-only drift decision log - autopilot decides at verify time; this is the audit trail.
export function useDriftLogQuery() {
  return useQuery({
    queryKey: qk.drift,
    queryFn: () => getJSON<DriftEvent[]>("/api/guard/drift"),
    refetchInterval: pollWhenOffline(60_000),
  });
}

// Web redesign v2: effective server config for the Settings page (read-only; change via env).
export function useConfigQuery() {
  return useQuery({
    queryKey: qk.config,
    queryFn: () => getJSON<ConfigEntry[]>("/api/config"),
    staleTime: 300_000,
  });
}

export function useDevicesTokensQuery() {
  return useQuery({
    queryKey: qk.devicesTokens,
    queryFn: () => getJSON<DeviceTokenMeta[]>("/api/devices/tokens"),
  });
}

export function useDevicesAuditQuery() {
  return useQuery({
    queryKey: qk.devicesAudit,
    queryFn: () => getJSON<AuditEvent[]>("/api/devices/audit"),
    refetchInterval: pollWhenOffline(60_000),
  });
}

export function useLedgerQuery(limit = 200) {
  return useQuery({
    queryKey: qk.ledger(limit),
    queryFn: () => getJSON<LedgerEntry[]>(`/api/ledger?limit=${limit}`),
    refetchInterval: 30_000,
  });
}

// P2.4: structural analysis of the memory graph (communities, hubs, bridges, cycles).
export function useArchitectureReportQuery() {
  return useQuery({
    queryKey: qk.architectureReport,
    queryFn: () => getJSON<ArchitectureReport>("/api/memory/architecture-report"),
    staleTime: 60_000,
  });
}

// P2.6: activity heatmap (last `days` days, default 365).
export function useHeatmapQuery(days = 365) {
  return useQuery({
    queryKey: qk.heatmap(days),
    queryFn: () =>
      getJSON<Record<string, number>>(`/api/memory/heatmap?days=${days}`),
    staleTime: 60_000,
  });
}

// v0.8.0 Sprint 5: memories in the [0.70, 0.90] promotion review band.
export function usePromotionCandidatesQuery() {
  return useQuery({
    queryKey: qk.promotionCandidates,
    queryFn: () => getJSON<Memory[]>("/api/memory/promotion-candidates"),
    refetchInterval: pollWhenOffline(60_000),
  });
}

// v0.8.0 Sprint 4/5/8/9: recent background-job runs (session-gc, memory-decay,
// access-log-prune, llm-intelligence, memory-demote, tune), optionally filtered to one job.
export function useCronHistoryQuery(job?: string) {
  return useQuery({
    queryKey: qk.cronHistory(job),
    queryFn: () =>
      getJSON<CronRun[]>(
        `/api/cron/history${job ? `?job=${encodeURIComponent(job)}` : ""}`,
      ),
    refetchInterval: 30_000,
  });
}

// v0.8.0 Sprint 4: every background job, its schedule, and its last run.
export function useCronJobsQuery() {
  return useQuery({
    queryKey: qk.cronJobs,
    queryFn: () => getJSON<CronJobStatus[]>("/api/cron/jobs"),
    refetchInterval: 30_000,
  });
}

// v0.8.0 Sprint 8: "while you were away" autopilot summary.
export function useAutopilotDigestQuery(hours = 24) {
  return useQuery({
    queryKey: qk.autopilotDigest(hours),
    queryFn: () => getJSON<AutopilotDigest>(`/api/memory/autopilot-digest?hours=${hours}`),
    refetchInterval: pollWhenOffline(60_000),
  });
}

// v0.8.0 Sprint 8: recent promotion/demotion events, auto and manual alike.
export function usePromotionLogQuery(limit = 50) {
  return useQuery({
    queryKey: qk.promotionLog(limit),
    queryFn: () => getJSON<PromotionLogEntry[]>(`/api/memory/promotion-log?limit=${limit}`),
    refetchInterval: 30_000,
  });
}

// v0.8.0 Sprint 3/10: every known project, enriched with memory_count + last_memory_at.
export function useProjectsQuery() {
  return useQuery({
    queryKey: qk.projects,
    queryFn: () => getJSON<ProjectWithStats[]>("/api/projects"),
    staleTime: 30_000,
  });
}

export function useProjectQuery(id: string) {
  return useQuery({
    queryKey: qk.project(id),
    queryFn: () => getJSON<ProjectWithStats>(`/api/projects/${encodeURIComponent(id)}`),
    enabled: id.length > 0,
  });
}

// v0.8.0 Sprint 10: every memory in an exact scope, no ranking, no Global blend - the "what
// does this project know" view on the Projects detail page.
export function useMemoryByScopeQuery(scopeType: "project" | "session", scopeId: string, limit = 50) {
  return useQuery({
    queryKey: qk.memoryByScope(scopeType, scopeId, limit),
    queryFn: () =>
      getJSON<Memory[]>(
        `/api/memory/by-scope?scope_type=${scopeType}&scope_id=${encodeURIComponent(scopeId)}&limit=${limit}`,
      ),
    enabled: scopeId.length > 0,
  });
}

// v0.8.0 Sprint 6: every ingested document, most-recently-updated first.
export function useDocumentsQuery() {
  return useQuery({
    queryKey: qk.documents,
    queryFn: () => getJSON<DocumentSummary[]>("/api/documents"),
    staleTime: 30_000,
  });
}

export function useDocumentSearchQuery(q: string, limit = 10) {
  return useQuery({
    queryKey: qk.documentSearch(q),
    queryFn: () =>
      getJSON<DocumentChunkRecord[]>(
        `/api/documents/search?limit=${limit}&q=${encodeURIComponent(q)}`,
      ),
    enabled: q.length > 0,
  });
}

export function useDeleteDocumentMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => delJSON<{ deleted: boolean }>(`/api/documents/${encodeURIComponent(id)}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.documents });
      toast("Document deleted");
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

// ---- mutations ---------------------------------------------------------------

function errMessage(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

export function useLoginMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: { username: string; password: string }) =>
      postJSON("/api/auth/login", input),
    onSuccess: async () => {
      const me = await getJSON<Me>("/api/auth/me").catch(() => null);
      if (me) useMeStore.getState().setMe(me);
      qc.invalidateQueries();
      toast.success(`Welcome back, ${me?.username ?? "admin"}`);
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function useSetupMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: { username: string; password: string }) =>
      postJSON("/api/auth/setup", input),
    onSuccess: async () => {
      const me = await getJSON<Me>("/api/auth/me").catch(() => null);
      if (me) useMeStore.getState().setMe(me);
      qc.invalidateQueries();
      toast.success(`Admin "${me?.username}" created`);
    },
    onError: (e) => {
      if (e instanceof ApiError && e.status === 409)
        toast.error("An admin already exists. Use the Sign in page instead.");
      else toast.error(errMessage(e));
    },
  });
}

export function useLogoutMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => postJSON("/api/auth/logout", {}),
    onSuccess: async () => {
      useMeStore.getState().clearMe();
      await qc.invalidateQueries({ queryKey: qk.me });
      qc.clear();
      toast("Signed out", { description: "Your session has been cleared." });
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

// v0.8.0 Sprint 5: approve a promotion candidate (-> Global scope, locked).
export function usePromoteMemoryMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => postJSON<Memory>(`/api/memory/${encodeURIComponent(id)}/promote`, {}),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.promotionCandidates });
      toast.success("Promoted to Global");
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

// v0.8.0 Sprint 5: dismiss a promotion candidate ("don't ask again").
export function useDismissPromotionMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      postJSON<Memory>(`/api/memory/${encodeURIComponent(id)}/dismiss-promotion`, {}),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.promotionCandidates });
      toast("Dismissed");
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

// v0.8.0 Sprint 8 (Undo): revert a promotion back to the scope it was promoted from. 404 means
// the memory either doesn't exist or was never promoted through this pipeline.
export function useDemoteMemoryMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => postJSON<Memory>(`/api/memory/${encodeURIComponent(id)}/demote`, {}),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["memory"] });
      toast.success("Reverted to prior scope");
    },
    onError: (e) => {
      if (e instanceof ApiError && e.status === 404) {
        toast.error("Nothing to undo - not tracked as promoted");
      } else {
        toast.error(errMessage(e));
      }
    },
  });
}

// v0.8.0 Sprint 4/9: manually trigger a background job now (same code path the scheduler uses).
export function useRunCronJobMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (job: string) =>
      postJSON<CronRun>(`/api/cron/run/${encodeURIComponent(job)}`, {}),
    onSuccess: (run) => {
      qc.invalidateQueries({ queryKey: qk.cronJobs });
      qc.invalidateQueries({ queryKey: ["cron", "history"] });
      toast.success(`Ran ${run.job}`, { description: run.detail });
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function useIssueTokenMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: IssueTokenInput) =>
      postJSON<IssuedToken>("/api/devices/tokens", {
        name: input.name,
        scope: input.scope,
        expires_in_days: input.expires_in_days === "" ? null : Number(input.expires_in_days),
      }),
    onSuccess: (t) => {
      qc.invalidateQueries({ queryKey: qk.devicesTokens });
      toast.success(`Issued ${t.scope} token for "${t.name}"`);
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function useRevokeTokenMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      postJSON(`/api/devices/tokens/${encodeURIComponent(id)}/revoke`, {}),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.devicesTokens });
      qc.invalidateQueries({ queryKey: qk.devicesAudit });
      toast("Token revoked");
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

// Web redesign v2: minimal housekeeping curation on the Memory Browser. Agents do the real
// curation via MCP; pin + delete survive as admin housekeeping (like document delete).
export function usePinMemoryMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, pinned }: { id: string; pinned: boolean }) =>
      postJSON<Memory>(`/api/memory/${encodeURIComponent(id)}/pin`, { pinned }),
    onSuccess: (m) => {
      qc.invalidateQueries({ queryKey: ["memory"] });
      toast(m.pinned ? "Pinned" : "Unpinned");
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function useDeleteMemoryMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => delJSON<{ deleted: boolean }>(`/api/memory/${encodeURIComponent(id)}`),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["memory"] });
      qc.invalidateQueries({ queryKey: qk.stats });
      toast("Memory deleted");
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function useGeneratePairCodeMutation() {
  return useMutation({
    mutationFn: (input: PairCodeInput) =>
      postJSON<PairCode>("/api/devices/pair-codes", input),
    onSuccess: (p) => {
      toast.success(`Pair code for "${p.name}" valid ${p.code.length} chars`);
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}
