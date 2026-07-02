import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  ApiError,
  API_BASE,
  delJSON,
  getJSON,
  postBinary,
  postJSON,
  type ArchitectureReport,
  type AuditEvent,
  type CompressionDemo,
  type CronRun,
  type DeviceTokenMeta,
  type Health,
  type IssuedToken,
  type LedgerEntry,
  type Me,
  type Memory,
  type PairCode,
  type RegistryRevocation,
  type RegistryTrustGrant,
  type ScoredMemory,
  type Stats,
} from "@/lib/api";
import { useMeStore } from "@/lib/stores/me";
import type {
  AnchorInput,
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
  cronHistory: ["cron", "history"] as const,
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
    refetchInterval: 10_000,
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
    refetchInterval: 30_000,
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
    refetchInterval: 5_000,
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
    refetchInterval: 30_000,
  });
}

// v0.8.0 Sprint 4/5: recent background-job runs (session-gc, memory-decay,
// access-log-prune, llm-intelligence).
export function useCronHistoryQuery() {
  return useQuery({
    queryKey: qk.cronHistory,
    queryFn: () => getJSON<CronRun[]>("/api/cron/history"),
    refetchInterval: 30_000,
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

export function usePublishPackMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: { tarball: ArrayBuffer | Uint8Array; trusted?: string }) =>
      postBinary<unknown>(
        "/api/registry/packs",
        input.tarball,
        "application/octet-stream",
      ),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: qk.registryPacks });
      await qc.invalidateQueries({ queryKey: qk.registryRevocations });
    },
  });
}

export function useRevokePackMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: { name: string; version: string }) =>
      delJSON<unknown>(`/api/registry/packs/${encodeURIComponent(input.name)}/${encodeURIComponent(input.version)}`),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: qk.registryPacks });
      await qc.invalidateQueries({ queryKey: qk.registryRevocations });
    },
  });
}

export function useAddTrustedKeyMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: { key: string; allows: string; label?: string }) =>
      postJSON<RegistryTrustGrant>("/api/registry/trusted-keys", input),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: qk.registryTrustedKeys });
      toast.success("Trusted key added");
    },
    onError: (e: unknown) => toast.error(errMessage(e)),
  });
}

export function useRemoveTrustedKeyMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (key: string) =>
      delJSON<unknown>(`/api/registry/trusted-keys?key=${encodeURIComponent(key)}`),
    onSuccess: async () => {
      await qc.invalidateQueries({ queryKey: qk.registryTrustedKeys });
      toast.success("Trusted key removed");
    },
    onError: (e: unknown) => toast.error(errMessage(e)),
  });
}

// ---- WebSocket status (P2.1) --------------------------------------------------

export type WsStatus = "connecting" | "connected" | "disconnected";

export interface UseWebSocketResult {
  status: WsStatus;
  /** Force a reconnect (e.g. after the user changes the API base). */
  reconnect: () => void;
}

const WS_EVENT_NAME = "cairn:ws-status";

function emitWsStatus(status: WsStatus) {
  if (typeof window === "undefined") return;
  window.dispatchEvent(new CustomEvent<WsStatus>(WS_EVENT_NAME, { detail: status }));
}

export function useWebSocket(): UseWebSocketResult {
  const [status, setStatus] = useState<WsStatus>("connecting");
  const [nonce, setNonce] = useState(0);
  useEffect(() => {
    if (typeof window === "undefined") return;
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<WsStatus>).detail;
      setStatus(detail);
    };
    window.addEventListener(WS_EVENT_NAME, handler);
    return () => window.removeEventListener(WS_EVENT_NAME, handler);
  }, []);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const base = (API_BASE || "").replace(/^http/, "ws");
    if (!base) return;
    const url = `${base}/api/ws`;
    let ws: WebSocket | null = null;
    let reconnectTimer: number | null = null;
    let cancelled = false;
    const open = () => {
      if (cancelled) return;
      emitWsStatus("connecting");
      try {
        ws = new WebSocket(url);
      } catch {
        emitWsStatus("disconnected");
        reconnectTimer = window.setTimeout(open, 3000);
        return;
      }
      ws.onopen = () => emitWsStatus("connected");
      ws.onclose = () => {
        emitWsStatus("disconnected");
        reconnectTimer = window.setTimeout(open, 3000);
      };
      ws.onerror = () => {
        emitWsStatus("disconnected");
      };
    };
    open();
    return () => {
      cancelled = true;
      if (reconnectTimer !== null) window.clearTimeout(reconnectTimer);
      ws?.close();
    };
  }, [nonce]);

  return { status, reconnect: () => setNonce((n) => n + 1) };
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

export function useSetAnchorMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: AnchorInput) => postJSON("/api/guard/anchor", input),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: qk.anchor });
      qc.invalidateQueries({ queryKey: qk.stats });
      toast.success("Anchor set");
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
