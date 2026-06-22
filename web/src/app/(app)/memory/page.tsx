"use client";

import { Suspense } from "react";
import { HubTabs, type HubTab } from "@/components/HubTabs";
import RememberPage from "./remember/page";
import RecallPage from "./recall/page";
import WakeupPage from "./wakeup/page";
import GraphPage from "./graph/page";
import InspectorPage from "./inspector/page";
import AssemblePage from "./assemble/page";
import SavingsPage from "./savings/page";

const MEMORY_TABS: HubTab[] = [
  { id: "remember", label: "Remember", content: <RememberPage /> },
  { id: "recall", label: "Recall", content: <RecallPage /> },
  { id: "wakeup", label: "Wakeup", content: <WakeupPage /> },
  { id: "graph", label: "Graph", content: <GraphPage /> },
  { id: "inspector", label: "Inspector", content: <InspectorPage /> },
  { id: "assemble", label: "Assemble", content: <AssemblePage /> },
  { id: "savings", label: "Savings", content: <SavingsPage /> },
];

export default function MemoryPage() {
  return (
    <Suspense fallback={null}>
      <HubTabs
        view="memory"
        title="Memory & Context"
        description="Write, recall, and read — the core loop. Plus the byte-savings ledger that proves it works."
        tabs={MEMORY_TABS}
        defaultTab="remember"
      />
    </Suspense>
  );
}