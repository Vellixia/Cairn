"use client";

import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { Info } from "lucide-react";
import { api } from "@/lib/api";
import {
  ErrorState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

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
    <div>
      <PageHeader
        title="Sync status"
        subtitle="What this server has received from linked machines"
      />

      {status.isLoading && <ListSkeleton rows={2} />}
      {status.error != null && <ErrorState error={status.error} />}

      {status.data && (
        <div className="space-y-4" data-testid="sync-status">
          <div className="grid gap-3 sm:grid-cols-2">
            <Card className="gap-0 p-4">
              <div className="text-2xl font-semibold tabular-nums">
                {status.data.applied_items}
              </div>
              <div className="text-muted-foreground text-xs">items applied</div>
            </Card>
            <Card className="gap-0 p-4">
              <div className="text-2xl font-semibold">
                {status.data.last_applied_at ? "Synced" : "Idle"}
              </div>
              <div className="text-muted-foreground text-xs">
                last applied {formatDate(status.data.last_applied_at)}
              </div>
            </Card>
          </div>

          <Alert>
            <Info />
            <AlertTitle>Pending work is not visible here</AlertTitle>
            <AlertDescription>
              Pending and failed counts live on each machine, because unsent
              work has by definition not reached this server. Run{" "}
              <code className="bg-muted rounded px-1 py-0.5 font-mono text-xs">
                cairn sync status
              </code>{" "}
              there to see them.
            </AlertDescription>
          </Alert>

          <Card>
            <CardHeader>
              <CardTitle className="text-sm font-medium">
                What the server keeps
              </CardTitle>
              <CardDescription>
                Tasks, sessions, memories and handoffs. Never raw observations —
                those stay on the machine that captured them.
              </CardDescription>
            </CardHeader>
            <CardContent />
          </Card>
        </div>
      )}
    </div>
  );
}
