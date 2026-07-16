"use client";

import { useQuery } from "@tanstack/react-query";
import { getJSON, type AuditEvent, type Stats } from "@/lib/api";
import { qk } from "@/lib/queries";
import { cn } from "@/lib/utils";
import {
  Activity,
  Anchor,
  KeyRound,
  LogIn,
  Settings,
  ShieldAlert,
  RefreshCcw,
  Sparkles,
  Brain,
} from "lucide-react";
import { useMemo, type ComponentType, type SVGProps } from "react";

type Kind =
  | "memory.remembered"
  | "memory.recalled"
  | "device.token_issued"
  | "device.token_revoked"
  | "admin.login_ok"
  | "admin.login_failed"
  | "admin.setup"
  | "anchor.set"
  | "reliability.drift_detected"
  | "sync.pulled"
  | "sync.pushed";

const KIND_META: Record<
  Kind,
  { label: string; icon: ComponentType<SVGProps<SVGSVGElement>>; tone: string }
> = {
  "memory.remembered": {
    label: "Memory stored",
    icon: Sparkles,
    tone: "text-[hsl(var(--color-positive))]",
  },
  "memory.recalled": {
    label: "Memory recalled",
    icon: Brain,
    tone: "text-[hsl(var(--color-info))]",
  },
  "device.token_issued": {
    label: "Device token issued",
    icon: KeyRound,
    tone: "text-[hsl(var(--color-info))]",
  },
  "device.token_revoked": {
    label: "Device token revoked",
    icon: KeyRound,
    tone: "text-[hsl(var(--color-warning))]",
  },
  "admin.login_ok": {
    label: "Admin signed in",
    icon: LogIn,
    tone: "text-[hsl(var(--color-positive))]",
  },
  "admin.login_failed": {
    label: "Admin sign-in failed",
    icon: LogIn,
    tone: "text-[hsl(var(--color-danger))]",
  },
  "admin.setup": {
    label: "Admin provisioned",
    icon: Settings,
    tone: "text-[hsl(var(--color-info))]",
  },
  "anchor.set": {
    label: "Anchor updated",
    icon: Anchor,
    tone: "text-[hsl(var(--color-info))]",
  },
  "reliability.drift_detected": {
    label: "Drift detected",
    icon: ShieldAlert,
    tone: "text-[hsl(var(--color-warning))]",
  },
  "sync.pulled": {
    label: "Sync pulled",
    icon: RefreshCcw,
    tone: "text-muted-foreground",
  },
  "sync.pushed": {
    label: "Sync pushed",
    icon: RefreshCcw,
    tone: "text-muted-foreground",
  },
};

const KNOWN_KINDS = new Set(Object.keys(KIND_META) as Kind[]);

function classifyAudit(ev: AuditEvent): Kind | "unknown" {
  const k = ev.kind.toLowerCase();
  if (k.includes("remember") || k === "memory.add" || k === "memory.create")
    return "memory.remembered";
  if (k.includes("recall") || k === "memory.read") return "memory.recalled";
  if (k.includes("token") && (k.includes("issue") || k.includes("create")))
    return "device.token_issued";
  if (k.includes("token") && (k.includes("revok") || k.includes("delete")))
    return "device.token_revoked";
  if (k.includes("login") && !k.includes("fail")) return "admin.login_ok";
  if (k.includes("login") && k.includes("fail")) return "admin.login_failed";
  if (k.includes("setup") || k.includes("provision")) return "admin.setup";
  if (k.includes("anchor")) return "anchor.set";
  if (k.includes("drift")) return "reliability.drift_detected";
  if (k.includes("sync") && k.includes("pull")) return "sync.pulled";
  if (k.includes("sync") && k.includes("push")) return "sync.pushed";
  return "unknown";
}

function relativeTime(ts: number): string {
  const now = Date.now();
  const diff = Math.max(0, now - ts);
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
}

export interface ActivityTimelineProps {
  limit?: number;
}

export function ActivityTimeline({ limit = 8 }: ActivityTimelineProps) {
  const auditQ = useQuery({
    queryKey: qk.devicesAudit,
    queryFn: () => getJSON<AuditEvent[]>("/api/devices/audit"),
  });
  const statsQ = useQuery({
    queryKey: qk.activityStats,
    queryFn: () => getJSON<Stats>("/api/stats"),
  });

  const items = useMemo(() => {
    const events = (auditQ.data ?? []).slice(0, limit);
    return events.map((ev) => {
      const k = classifyAudit(ev);
      const meta = k === "unknown" ? null : KIND_META[k as Kind];
      return { ev, k, meta };
    });
  }, [auditQ.data, limit]);

  const isLoading = auditQ.isLoading || statsQ.isLoading;
  const isError = auditQ.isError;

  return (
    <div className="space-y-3">
      <header className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Activity className="size-4 text-muted-foreground" aria-hidden="true" />
          <h2 className="text-sm font-semibold tracking-tight">Recent activity</h2>
        </div>
        {statsQ.data ? (
          <span className="text-xs text-muted-foreground">
            {statsQ.data.memories.toLocaleString()} memories stored
          </span>
        ) : null}
      </header>
      {isLoading ? (
        <div className="cairn-skeleton h-32" aria-label="Loading activity" />
      ) : isError ? (
        <p className="text-sm text-muted-foreground">
          Couldn&apos;t load the activity ledger.
        </p>
      ) : items.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          No recent activity yet. New events appear here as you remember, recall, or issue tokens.
        </p>
      ) : (
        <ol className="space-y-2">
          {items.map((it, i) => (
            <li
              key={`${it.ev.ts}-${i}`}
              className="flex items-start gap-3 rounded-md border border-line/40 bg-muted/30 px-3 py-2"
            >
              <div className={cn("mt-0.5", it.meta?.tone ?? "text-muted-foreground")}>
                {it.meta ? (
                  <it.meta.icon className="size-4" aria-hidden="true" />
                ) : (
                  <Activity className="size-4" aria-hidden="true" />
                )}
              </div>
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium leading-tight">
                  {it.meta?.label ?? it.ev.kind}
                </p>
                <p className="truncate text-xs text-muted-foreground">
                  {it.ev.detail || it.ev.actor}
                </p>
              </div>
              <span className="shrink-0 text-xs text-muted-foreground">
                {/* AuditEvent.ts is unix SECONDS (admin.rs `Utc::now().timestamp()`) */}
                {relativeTime(it.ev.ts * 1000)}
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
