"use client";

import { useEffect, useState } from "react";
import { useRouter, usePathname } from "next/navigation";
import { getJSON, type AuthStatus, type Me } from "@/lib/api";

/**
 * SessionGate: on mount, probes `/api/auth/status` + `/api/auth/me` (public + cookie-based).
 *
 * - If status reports `setup_required`, redirect to /setup.
 * - Else, if `me` returns 401 (no cookie or invalid cookie), redirect to /login?from=… .
 * - Else, render children.
 *
 * The first paint uses a CSS-only skeleton so we don't flash the dashboard before the probe
 * completes (and we don't accidentally leak auth state via a brief login redirect).
 */

type Phase =
  | { kind: "loading" }
  | { kind: "ready"; me: Me }
  | { kind: "setup"; authStatus: AuthStatus };

export function SessionGate({ children }: { children: React.ReactNode }) {
  const router = useRouter();
  const pathname = usePathname();
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await getJSON<AuthStatus>("/api/auth/status");
        if (cancelled) return;
        if (status.setup_required) {
          setPhase({ kind: "setup", authStatus: status });
          router.replace("/setup");
          return;
        }
        const me = await getJSON<Me>("/api/auth/me");
        if (cancelled) return;
        setPhase({ kind: "ready", me });
      } catch {
        if (cancelled) return;
        const from = encodeURIComponent(pathname || "/dashboard");
        router.replace(`/login?from=${from}`);
      }
    })();
    return () => { cancelled = true; };
  }, [router, pathname]);

  if (phase.kind === "ready") return <>{children}</>;
  return <SessionGateSkeleton />;
}

function SessionGateSkeleton() {
  return (
    <div className="min-h-screen flex">
      <aside className="w-60 shrink-0 border-r border-line bg-surface/60 p-4 space-y-3">
        <div className="cairn-skeleton h-8 w-32" />
        <div className="cairn-skeleton h-5 w-24 mt-4" />
        <div className="cairn-skeleton h-5 w-28" />
        <div className="cairn-skeleton h-5 w-20" />
        <div className="cairn-skeleton h-5 w-24" />
        <div className="cairn-skeleton h-5 w-28 mt-4" />
        <div className="cairn-skeleton h-5 w-24" />
      </aside>
      <main className="flex-1 p-8 space-y-4">
        <div className="cairn-skeleton h-9 w-48" />
        <div className="grid gap-4 md:grid-cols-2">
          <div className="cairn-skeleton h-32" />
          <div className="cairn-skeleton h-32" />
        </div>
        <div className="cairn-skeleton h-24" />
      </main>
    </div>
  );
}
