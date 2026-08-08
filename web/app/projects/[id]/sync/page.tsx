"use client";

import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Card, Empty, ErrorNote, PageHeader, ProjectNav } from "@/components/ui";

export default function SyncPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const status = useQuery({
    queryKey: ["sync", id],
    queryFn: () => api.syncStatus(id),
  });

  return (
    <main>
      <PageHeader
        title="Sync status"
        subtitle="What this server has received from linked machines"
      />
      <ProjectNav id={id} active="sync" />

      {status.error != null && <ErrorNote error={status.error} />}
      {status.isLoading && <Empty>Loading…</Empty>}

      {status.data && (
        <div className="space-y-3" data-testid="sync-status">
          <Card>
            <div className="text-2xl font-semibold">
              {status.data.applied_items}
            </div>
            <div className="text-xs uppercase tracking-wide text-neutral-500">
              items applied
            </div>
          </Card>
          <Card>
            <div className="text-sm">
              Last applied:{" "}
              {status.data.last_applied_at
                ? new Date(status.data.last_applied_at).toLocaleString()
                : "never"}
            </div>
          </Card>
          <p className="text-sm text-neutral-500">
            Pending and failed counts live on each machine. Run{" "}
            <code>cairn sync status</code> there to see them.
          </p>
        </div>
      )}
    </main>
  );
}
