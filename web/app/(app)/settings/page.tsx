"use client";

import SettingsTokens from "@/components/settings/tokens";
import SettingsSystem from "@/components/settings/system";
import SettingsUsers from "@/components/settings/users";
import { PageHeader } from "@/components/page";

export default function SettingsPage() {
  return (
    <div className="space-y-10">
      <PageHeader title="Settings" subtitle="Account, projects, access, and server controls" />
      <SettingsTokens />
      <SettingsUsers />
      <SettingsSystem />
      <p className="text-muted-foreground text-sm">Privacy policy and logical import/export are unavailable in this build.</p>
    </div>
  );
}
