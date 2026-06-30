"use client";

import { Suspense } from "react";
import { HubTabs, type HubTab } from "@/components/HubTabs";
import RecallPage from "./recall/page";
import WakeupPage from "./wakeup/page";
import GraphPage from "./graph/page";
import SavingsPage from "./savings/page";
import CompressionPage from "./compression/page";
import ArchitecturePage from "./architecture/page";
import HeatmapPage from "./heatmap/page";

const MEMORY_TABS: HubTab[] = [
  { id: "wakeup", label: "Wakeup", content: <WakeupPage /> },
  { id: "recall", label: "Recall", content: <RecallPage /> },
  { id: "graph", label: "Graph", content: <GraphPage /> },
  { id: "compression", label: "Compression Lab", content: <CompressionPage /> },
  { id: "savings", label: "Savings", content: <SavingsPage /> },
  { id: "architecture", label: "Architecture", content: <ArchitecturePage /> },
  { id: "heatmap", label: "Activity", content: <HeatmapPage /> },
];

export default function MemoryPage() {
  return (
    <Suspense fallback={null}>
      <HubTabs
        view="memory"
        title="Memory & Context"
        description="Recall and explore what Cairn has stored. The agent writes; this is where you watch."
        tabs={MEMORY_TABS}
        defaultTab="wakeup"
      />
    </Suspense>
  );
}
