import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  ApiError,
  delJSON,
  getJSON,
  postJSON,
  type AuditEvent,
  type Checkpoint,
  type DeviceTokenMeta,
  type Health,
  type IssuedToken,
  type Me,
  type Memory,
  type PairCode,
  type Pool,
  type ReadResult,
  type RollbackReport,
  type Sanitized,
  type ScoredMemory,
  type ShareExport,
  type Stats,
} from "@/lib/api";
import { useMeStore } from "@/lib/stores/me";
import type {
  AnchorInput,
  AssembleInput,
  CheckpointInput,
  ContextReadInput,
  IssueTokenInput,
  PairCodeInput,
  RecallInput,
  RememberInput,
  SanitizeInput,
} from "@/lib/forms/schemas";

// ---- query keys (single source of truth) ------------------------------------

export const qk = {
  health: ["health"] as const,
  me: ["auth", "me"] as const,
  stats: ["stats"] as const,
  anchor: ["guard", "anchor"] as const,
  anchorList: (path: string) => ["guard", "anchor", path] as const,
  memories: (limit: number) => ["memory", "wakeup", limit] as const,
  recall: (q: string) => ["memory", "recall", q] as const,
  context: (path: string, mode: string) => ["context", "read", path, mode] as const,
  contextExpand: (hash: string) => ["context", "expand", hash] as const,
  checkpoints: ["guard", "checkpoints"] as const,
  devicesTokens: ["devices", "tokens"] as const,
  devicesAudit: ["devices", "audit"] as const,
  pool: ["pool"] as const,
  shareExport: ["share", "export"] as const,
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

export function useContextReadQuery(input: ContextReadInput | null) {
  return useQuery({
    queryKey: input ? qk.context(input.path, input.mode) : ["context", "read", "_", "_"],
    queryFn: () =>
      getJSON<ReadResult>(
        `/api/context/read?path=${encodeURIComponent(input!.path)}&mode=${encodeURIComponent(input!.mode)}`,
      ),
    enabled: !!input,
  });
}

export function useCheckpointsQuery() {
  return useQuery({
    queryKey: qk.checkpoints,
    queryFn: () => getJSON<Checkpoint[]>("/api/guard/checkpoints"),
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

export function usePoolQuery() {
  return useQuery({
    queryKey: qk.pool,
    queryFn: () => getJSON<Pool>("/api/pool"),
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

export function useRememberMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: RememberInput) =>
      postJSON<Memory>("/api/memory", input),
    onSuccess: (m) => {
      qc.invalidateQueries({ queryKey: ["memory"] });
      qc.invalidateQueries({ queryKey: qk.stats });
      toast.success(`stored ${m.kind}/${m.tier} · ${m.id.slice(0, 8)}`);
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

export function useCreateCheckpointMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: CheckpointInput) => {
      const q = input.label?.trim() ? `?label=${encodeURIComponent(input.label.trim())}` : "";
      return postJSON<Checkpoint>(`/api/guard/checkpoint${q}`, {});
    },
    onSuccess: (cp) => {
      qc.invalidateQueries({ queryKey: qk.checkpoints });
      qc.invalidateQueries({ queryKey: qk.stats });
      toast.success(`Checkpoint ${cp.id.slice(0, 8)} · ${cp.files} files`);
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function useRollbackMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      postJSON<RollbackReport>(`/api/guard/rollback?id=${encodeURIComponent(id)}`, {}),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: qk.checkpoints });
      toast(`Restored ${r.restored.length} · skipped ${r.skipped.length}`);
    },
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function useSanitizeMutation() {
  return useMutation({
    mutationFn: (input: SanitizeInput) => postJSON<Sanitized>("/api/share/sanitize", input),
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function useBuildExportMutation() {
  return useMutation({
    mutationFn: () => getJSON<ShareExport>("/api/share/export"),
    onError: (e) => toast.error(errMessage(e)),
  });
}

export function usePublishPoolMutation() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: async () => {
      const bundle = await getJSON<ShareExport>("/api/share/export");
      return postJSON<{ accepted: number; rejected: number }>(
        "/api/pool/contribute",
        bundle,
      );
    },
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: qk.pool });
      toast.success(`Published · ${r.accepted} accepted, ${r.rejected} rejected`);
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
