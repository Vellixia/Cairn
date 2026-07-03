import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  ApiError,
  delJSON,
  getJSON,
  postJSON,
  type ArchitectureReport,
  type AuditEvent,
  type AutopilotDigest,
  type CompressionDemo,
  type CronJobStatus,
  type CronRun,
  type DeviceTokenMeta,
  type Health,
  type IssuedToken,
  type LedgerEntry,
  type Me,
  type Memory,
  type PairCode,
  type DocumentSummary,
  type DocumentChunkRecord,
  type PromotionLogEntry,
  type ProjectWithStats,
  type RegistryRevocation,
  type RegistryTrustGrant,
  type ScoredMemory,
  type Stats,
} from "@/lib/api";
import { useMeStore } from "@/lib/stores/me";
import { pollWhenOffline } from "@/lib/stores/events";
import type {
  IssueTokenInput,
  PairCodeInput,
  RecallInput,
} from "@/lib/forms/schemas";

// ---- query keys (single source of truth) ------------------------------------

export const qk = {
  health: ["health"] as const,
  me: ["auth", "me"] as const,
  stats: ["stats"] as const,
  anchor: ["guard", "anchor"] as const,
  memories: (limit: number) => ["memory", "wakeup", limit] as const,
  recall: (q: string) => ["memory", "recall", q] as const,
  devicesTokens: ["devices", "tokens"] as const,
  devicesAudit: ["devices", "audit"] as const,
  ledger: (limit: number) => ["ledger", limit] as const,
  heatmap: (days: number) => ["memory", "heatmap", days] as const,
  architectureReport: ["memory", "architecture-report"] as const,
  registryPacks: ["registry", "packs"] as const,
  registryPack: (name: string) => ["registry", "packs", name] as const,
  registrySearch: (q: string) => ["registry", "search", q] as const,
  registryRevocations: ["registry", "revocations"] as const,
  registryTrustedKeys: ["registry", "trusted-keys"] as const,
  promotionCandidates: ["memory", "promotion-candidates"] as const,
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

export function useRecallQuery(q: string) {
  return useQuery({
    queryKey: qk.recall(q),
    queryFn: () => getJSON<ScoredMemory[]>(`/api/memory/recall?limit=20&q=${encodeURIComponent(q)}`),
    enabled: q.length > 0,
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

// P2.3: side-by-side compression demo (all 4 read modes for one file).
export function useCompressionDemoQuery(path: string | null) {
  return useQuery({
    queryKey: ["context", "compression-demo", path ?? ""],
    queryFn: () =>
      getJSON<CompressionDemo>(
        `/api/context/compression-demo?path=${encodeURIComponent(path ?? "")}`,
      ),
    enabled: !!path && path.length > 0,
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

// P2.8: registry dashboard.
export function useRegistryPacksQuery() {
  return useQuery({
    queryKey: qk.registryPacks,
    queryFn: () => getJSON<unknown[]>("/api/registry/packs"),
    staleTime: 30_000,
  });
}

export function useRegistryRevocationsQuery() {
  return useQuery({
    queryKey: qk.registryRevocations,
    queryFn: () => getJSON<RegistryRevocation[]>("/api/registry/revocations"),
    staleTime: 30_000,
  });
}

export function useRegistryTrustedKeysQuery() {
  return useQuery({
    queryKey: qk.registryTrustedKeys,
    queryFn: () => getJSON<RegistryTrustGrant[]>("/api/registry/trusted-keys"),
    staleTime: 30_000,
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
