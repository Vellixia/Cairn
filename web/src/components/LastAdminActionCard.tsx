"use client";

import { useDevicesAuditQuery } from "@/lib/queries";
import { Badge } from "@/components/ui/badge";

function relativeTime(ts: number, now: number): string {
  const diffMs = now - ts;
  const s = Math.round(diffMs / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  const d = Math.round(h / 24);
  return `${d}d ago`;
}

export function LastAdminActionCard({ className }: { className?: string }) {
  const { data, isLoading } = useDevicesAuditQuery();
  const latest = data?.[0];

  if (isLoading) {
    return (
      <div className={className} aria-hidden="true">
        <div className="h-12 w-full animate-pulse rounded bg-muted" />
      </div>
    );
  }
  if (!latest) {
    return (
      <div className={className}>
        <p className="text-xs text-muted-foreground">No admin actions recorded yet.</p>
      </div>
    );
  }
  return (
    <div className={className}>
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-foreground">
            <span className="font-mono text-xs text-muted-foreground">{latest.actor}</span>
            <span className="mx-1.5 text-muted-foreground">·</span>
            <span>{latest.kind.replace(/_/g, " ")}</span>
          </p>
          <p className="truncate text-xs text-muted-foreground">{latest.detail}</p>
        </div>
        <Badge variant="outline" className="shrink-0 font-mono text-[10px]">
          {relativeTime(latest.ts, Date.now())}
        </Badge>
      </div>
    </div>
  );
}
