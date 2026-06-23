"use client";

import { Suspense } from "react";
import { HubTabs, type HubTab } from "@/components/HubTabs";
import ScorePage from "./score/page";
import AnchorPage from "./anchor/page";
import CheckpointsPage from "./checkpoints/page";
import DriftPage from "./drift/page";
import SanitizePage from "./sanitize/page";
import RegistryPage from "./registry/page";
import PoolPage from "./pool/page";

const TRUST_TABS: HubTab[] = [
  { id: "score", label: "Score", content: <ScorePage /> },
  { id: "anchor", label: "Anchor", content: <AnchorPage /> },
  { id: "checkpoints", label: "Checkpoints", content: <CheckpointsPage /> },
  { id: "drift", label: "Drift", content: <DriftPage /> },
  { id: "sanitize", label: "Sanitize", content: <SanitizePage /> },
  { id: "registry", label: "Registry", content: <RegistryPage /> },
  { id: "pool", label: "Pool", content: <PoolPage /> },
];

export default function TrustPage() {
  return (
    <Suspense fallback={null}>
      <HubTabs
        view="trust"
        title="Trust"
        description="Reliability, the drift center, and the way memories leave this Cairn."
        tabs={TRUST_TABS}
        defaultTab="score"
      />
    </Suspense>
  );
}