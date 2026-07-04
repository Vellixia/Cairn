"use client";

import { useQuery } from "@tanstack/react-query";
import { getJSON, type Health, type Stats } from "@/lib/api";
import { cn } from "@/lib/utils";
import {
  CheckCircle2,
  CircleDashed,
  CircleSlash,
  Cpu,
  Database,
  HardDrive,
  Radio,
  Smartphone,
  AlertTriangle,
} from "lucide-react";
import { useEffect, useState, type ComponentType, type SVGProps } from "react";

type Status = "ok" | "warn" | "down" | "loading";

const TONE: Record<Status, string> = {
  ok: "border-[hsl(var(--color-positive))]/40 bg-[hsl(var(--color-positive))]/10 text-[hsl(var(--color-positive))]",
  warn: "border-[hsl(var(--color-warning))]/40 bg-[hsl(var(--color-warning))]/10 text-[hsl(var(--color-warning))]",
  down: "border-[hsl(var(--color-danger))]/40 bg-[hsl(var(--color-danger))]/10 text-[hsl(var(--color-danger))]",
  loading: "border-border bg-muted text-muted-foreground",
};

function statusToTone(s: Status) {
  return TONE[s];
}

function StatusIcon({ s }: { s: Status }) {
  if (s === "ok") return <CheckCircle2 className="size-3.5" aria-hidden="true" />;
  if (s === "warn") return <AlertTriangle className="size-3.5" aria-hidden="true" />;
  if (s === "down") return <CircleSlash className="size-3.5" aria-hidden="true" />;
  return <CircleDashed className="size-3.5 animate-pulse" aria-hidden="true" />;
}

function Pill({
  label,
  status,
  detail,
  icon: Icon,
}: {
  label: string;
  status: Status;
  detail?: string;
  icon: ComponentType<SVGProps<SVGSVGElement>>;
}) {
  return (
    <div
      className={cn(
        "inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-xs font-medium",
        statusToTone(status),
      )}
      title={detail ?? label}
    >
      <Icon className="size-3.5" aria-hidden="true" />
      <span>{label}</span>
      <StatusIcon s={status} />
    </div>
  );
}

function useServiceWorkerStatus(): Status {
  const [status, setStatus] = useState<Status>("loading");
  useEffect(() => {
    if (typeof navigator === "undefined" || !("serviceWorker" in navigator)) {
      setStatus("down");
      return;
    }
    let cancelled = false;
    navigator.serviceWorker
      .getRegistration()
      .then((reg) => {
        if (cancelled) return;
        setStatus(reg ? "ok" : "warn");
      })
      .catch(() => {
        if (cancelled) return;
        setStatus("down");
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return status;
}

const HEALTH_TTL = 30_000;

export function HealthRow() {
  const serverQ = useQuery({
    queryKey: ["health", "server"],
    queryFn: () => getJSON<Health>("/api/health"),
    refetchInterval: HEALTH_TTL,
    staleTime: HEALTH_TTL,
  });
  const setupQ = useQuery({
    queryKey: ["health", "setup"],
    queryFn: () => getJSON<{ ok: boolean; memories: number; detail?: string }>("/api/setup/health"),
    refetchInterval: HEALTH_TTL,
    staleTime: HEALTH_TTL,
  });
  const statsQ = useQuery({
    queryKey: ["health", "stats"],
    queryFn: () => getJSON<Stats>("/api/stats"),
    refetchInterval: HEALTH_TTL,
    staleTime: HEALTH_TTL,
  });

  const sw = useServiceWorkerStatus();

  const serverStatus: Status = serverQ.isLoading
    ? "loading"
    : serverQ.isError
      ? "down"
      : serverQ.data?.status === "ok"
        ? "ok"
        : "warn";

  const dbStatus: Status = setupQ.isLoading
    ? "loading"
    : setupQ.isError
      ? "down"
      : setupQ.data?.ok
        ? "ok"
        : "warn";

  const memoriesStatus: Status = statsQ.isLoading
    ? "loading"
    : statsQ.isError
      ? "down"
      : statsQ.data && typeof statsQ.data.memories === "number"
        ? "ok"
        : "warn";

  const reliabilityScore = statsQ.data?.reliability?.score;
  const reliabilityPct = typeof reliabilityScore === "number" ? reliabilityScore : undefined;
  const reliabilityStatus: Status =
    typeof reliabilityPct === "number"
      ? reliabilityPct >= 90
        ? "ok"
        : reliabilityPct >= 70
          ? "warn"
          : "down"
      : statsQ.isLoading
        ? "loading"
        : "warn";

  return (
    <div className="flex flex-wrap items-center gap-2" aria-label="System health">
      <Pill
        label="Server"
        status={serverStatus}
        detail={serverQ.data ? `${serverQ.data.name} ${serverQ.data.version}` : "API not reachable"}
        icon={Radio}
      />
      <Pill
        label="Database"
        status={dbStatus}
        detail={
          setupQ.data
            ? `${setupQ.data.memories} memories indexed`
            : "SurrealDB - graph + vector store"
        }
        icon={Database}
      />
      <Pill
        label="Embedder"
        status="ok"
        detail="local hashing (default)"
        icon={Cpu}
      />
      <Pill
        label="Reliability"
        status={reliabilityStatus}
        detail={
          typeof reliabilityPct === "number"
            ? `${Math.round(reliabilityPct)}% over last samples`
            : "no recent samples"
        }
        icon={HardDrive}
      />
      <Pill
        label="PWA"
        status={sw}
        detail="service worker"
        icon={Smartphone}
      />
    </div>
  );
}
