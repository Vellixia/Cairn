"use client";

import { useEffect } from "react";
import { SessionGate } from "@/components/SessionGate";
import { CairnSidebar } from "@/components/Sidebar";
import { Topbar } from "@/components/Topbar";
import { CommandPalette } from "@/components/CommandPalette";
import { Shortcuts } from "@/components/Shortcuts";
import { SidebarProvider } from "@/components/ui/sidebar";

/**
 * Dashboard shell. Wraps everything in <SessionGate>, which probes auth + redirects unauth'd
 * users to /login. <SidebarProvider> establishes the flex row that holds the sidebar and the
 * main content side-by-side (matches shadcn sidebar pattern). The command palette and
 * shortcuts modal are mounted here so they're available on every dashboard page.
 *
 * v0.5.0 Sprint 20: also registers the service worker (PWA) on first paint. The SW is
 * a no-op when the page is opened without HTTPS (browsers require secure context) --- the
 * dashboard itself still works, just without offline cache + push.
 */
export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  useEffect(() => {
    if (typeof window === "undefined") return;
    if (!("serviceWorker" in navigator)) return;
    // Register the SW only on HTTPS or localhost; browsers reject insecure contexts.
    if (window.location.protocol !== "https:" && window.location.hostname !== "localhost" && window.location.hostname !== "127.0.0.1") {
      return;
    }
    navigator.serviceWorker.register("/sw.js").catch(() => {
      // SW registration is best-effort; dashboard still works without it.
    });
  }, []);

  return (
    <SessionGate>
      <SidebarProvider>
        <CairnSidebar />
        <div className="flex-1 flex min-w-0 flex-col">
          <Topbar />
          <main className="flex-1 px-5 py-6 md:px-8 md:py-8 max-w-[1400px] w-full mx-auto">
            {children}
          </main>
        </div>
      </SidebarProvider>
      <CommandPalette />
      <Shortcuts />
    </SessionGate>
  );
}
