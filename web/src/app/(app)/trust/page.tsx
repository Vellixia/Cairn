"use client";

import { Suspense } from "react";
import { HubTabs, type HubTab } from "@/components/HubTabs";
import ScorePage from "./score/page";
import DriftPage from "./drift/page";

const TRUST_TABS: HubTab[] = [
  { id: "score", label: "Score", content: <ScorePage /> },
  { id: "drift", label: "Drift", content: <DriftPage /> },
];

export default function TrustPage() {
  return (
    <Suspense fallback={null}>
      <HubTabs
        view="trust"
        title="Trust"
        description="Reliability score and drift samples --- the agent maintains checkpoints and anchors."
        tabs={TRUST_TABS}
        defaultTab="score"
      />
    </Suspense>
  );
}
