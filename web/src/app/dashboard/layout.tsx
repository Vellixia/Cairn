"use client";

import { SessionGate } from "@/components/SessionGate";
import { CairnSidebar } from "@/components/Sidebar";
import { Topbar } from "@/components/Topbar";
import { CommandPalette } from "@/components/CommandPalette";
import { Shortcuts } from "@/components/Shortcuts";

/**
 * Dashboard shell. Wraps everything in <SessionGate>, which probes auth + redirects unauth'd
 * users to /login. Renders the sidebar (flat, non-collapsible) + sticky topbar + the active
 * section via `children`. The command palette and shortcuts modal are mounted here so they're
 * available on every dashboard page.
 */
export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <SessionGate>
      <div className="min-h-screen flex">
        <CairnSidebar />
        <div className="flex-1 flex min-w-0 flex-col">
          <Topbar />
          <main className="flex-1 px-5 py-6 md:px-8 md:py-8 max-w-[1400px] w-full mx-auto">
            {children}
          </main>
        </div>
      </div>
      <CommandPalette />
      <Shortcuts />
    </SessionGate>
  );
}
