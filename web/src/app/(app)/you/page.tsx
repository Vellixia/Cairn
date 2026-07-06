"use client";

import { Suspense } from "react";
import { HubTabs, type HubTab } from "@/components/HubTabs";
import ProfilePage from "./profile/page";
import TokensPage from "./tokens/page";
import AuditPage from "./audit/page";
import SessionsPage from "./sessions/page";
import SettingsPage from "./settings/page";

const YOU_TABS: HubTab[] = [
  { id: "profile", label: "Profile", content: <ProfilePage /> },
  { id: "tokens", label: "Tokens", content: <TokensPage /> },
  { id: "audit", label: "Audit", content: <AuditPage /> },
  { id: "sessions", label: "Sessions", content: <SessionsPage /> },
  { id: "settings", label: "Settings", content: <SettingsPage /> },
];

export default function YouPage() {
  return (
    <Suspense fallback={null}>
      <HubTabs
        view="you"
        title="You"
        description="Your profile, your devices, your sessions."
        tabs={YOU_TABS}
        defaultTab="profile"
      />
    </Suspense>
  );
}