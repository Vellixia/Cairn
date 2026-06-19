"use client";

import { useEffect, useState } from "react";
import { Sidebar } from "@/components/Sidebar";
import { SessionGate } from "@/components/SessionGate";
import { ToastTray } from "@/components/Toast";
import { getJSON, type Me } from "@/lib/api";

/**
 * Dashboard shell. Wraps everything in <SessionGate>, which probes auth + redirects unauth'd
 * users to /login. Renders the sidebar (collapsible below md), a sticky topbar, and the active
 * section via `children`.
 */
export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <SessionGate>
      <DashboardShell>{children}</DashboardShell>
    </SessionGate>
  );
}

function DashboardShell({ children }: { children: React.ReactNode }) {
  const [me, setMe] = useState<Me | null>(null);
  useEffect(() => {
    getJSON<Me>("/api/auth/me").then(setMe).catch(() => setMe(null));
  }, []);
  return (
    <div className="min-h-screen flex">
      <Sidebar />
      <div className="flex-1 flex min-w-0 flex-col">
        {me && <TopbarBootstrap me={me} />}
        <main className="flex-1 px-5 py-6 md:px-8 md:py-8 max-w-[1400px] w-full mx-auto">
          {children}
        </main>
      </div>
      <ToastTray />
    </div>
  );
}

function TopbarBootstrap({ me }: { me: Me }) {
  // Lazy import to avoid bundling the Topbar's polling in the SessionGate path.
  const [Topbar, setTopbar] = useState<React.ComponentType<{ me: Me }> | null>(null);
  useEffect(() => {
    import("@/components/Topbar").then((m) => setTopbar(() => m.Topbar));
  }, []);
  if (!Topbar) return <div className="h-14 border-b border-line" />;
  return <Topbar me={me} />;
}
