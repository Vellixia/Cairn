"use client";

import { useRouter, usePathname } from "next/navigation";
import { useEffect } from "react";
import { Skeleton } from "@/components/ui/skeleton";
import { useMeQuery } from "@/lib/queries";
import { useMeStore } from "@/lib/stores/me";
import { ApiError, getJSON, type AuthStatus } from "@/lib/api";

type Phase = "loading" | "ready" | "needs-setup" | "needs-login";

/**
 * SessionGate — auth gate for the dashboard layout.
 *
 * - Probes `/api/auth/status` + `/api/auth/me` on first mount.
 * - If setup is required, redirect to `/setup`.
 * - If me returns 401 (no cookie / invalid cookie), redirect to `/login?from=…`.
 * - Otherwise, render children and expose the `me` record via the `useMeStore` zustand store.
 *
 * The probe runs only on mount; client-side navigations within the dashboard layout do not
 * re-trigger it. After the probe succeeds the me record is also picked up by `useMeQuery`
 * so react-query keeps it fresh.
 */
export function SessionGate({ children }: { children: React.ReactNode }) {
  const router = useRouter();
  const pathname = usePathname();
  const setMe = useMeStore((s) => s.setMe);
  const me = useMeStore((s) => s.me);
  const meQuery = useMeQuery();

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await getJSON<AuthStatus>("/api/auth/status");
        if (cancelled) return;
        if (status.setup_required) {
          router.replace("/setup");
          return;
        }
        const me = await getJSON<{
          username: string;
          generation: number;
          login_at: number;
          expires_at: number;
        }>("/api/auth/me");
        if (cancelled) return;
        setMe(me);
      } catch {
        if (cancelled) return;
        const from = encodeURIComponent(pathname || "/dashboard");
        router.replace(`/login?from=${from}`);
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Derive the current phase from the me store and the react-query cache.
  // Once the probe on mount has succeeded (or react-query has caught up), `me` will be set
  // and we render the dashboard. Otherwise the skeleton.
  const phase: Phase = me
    ? "ready"
    : meQuery.error instanceof ApiError && meQuery.error.status === 401
      ? "needs-login"
      : meQuery.error
        ? "needs-login"
        : meQuery.isFetched && !meQuery.data
          ? "needs-login"
          : meQuery.data
            ? "ready"
            : "loading";

  if (phase === "ready") {
    return <>{children}</>;
  }
  return <SessionGateSkeleton />;
}

function SessionGateSkeleton() {
  return (
    <div className="min-h-screen flex">
      <aside className="w-60 shrink-0 border-r border-line p-4 space-y-3">
        <Skeleton className="h-8 w-32" />
        <div className="space-y-2 mt-4">
          <Skeleton className="h-5 w-24" />
          <Skeleton className="h-5 w-28" />
          <Skeleton className="h-5 w-20" />
          <Skeleton className="h-5 w-24" />
        </div>
        <div className="space-y-2 mt-4">
          <Skeleton className="h-5 w-28" />
          <Skeleton className="h-5 w-24" />
        </div>
      </aside>
      <main className="flex-1 p-8 space-y-4">
        <Skeleton className="h-9 w-48" />
        <div className="grid gap-4 md:grid-cols-2">
          <Skeleton className="h-32" />
          <Skeleton className="h-32" />
        </div>
        <Skeleton className="h-24" />
      </main>
    </div>
  );
}
