import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  ApiError,
  getJSON,
  postJSON,
  type AuditEvent,
  type DeviceTokenMeta,
  type Health,
  type IssuedToken,
  type LedgerEntry,
  type Me,
  type Memory,
  type PairCode,
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
