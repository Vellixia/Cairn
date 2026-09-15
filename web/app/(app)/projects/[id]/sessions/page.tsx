"use client";

import { use } from "react";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import {
  EmptyState,
  ErrorState,
  ListSkeleton,
  PageHeader,
} from "@/components/page";
import { SessionRow } from "@/components/session";
import { Button } from "@/components/ui/button";

export default function SessionsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const sessions = useQuery({
    queryKey: ["sessions", id],
    queryFn: () => api.sessions(id),
  });

  return (
    <div>
      <PageHeader
        title="Sessions & Replay"
        subtitle="Accepted sessions, newest first. Replay remains read-only."
      >
        <Button variant="outline" size="sm" data-testid="session-replay-entry" render={<Link href={`/projects/${id}#activity`}>Replay accepted activity</Link>} />
      </PageHeader>

      {sessions.isLoading && <ListSkeleton />}
      {sessions.error != null && <ErrorState error={sessions.error} />}
      {sessions.data?.sessions.length === 0 && (
        <EmptyState
          title="No sessions yet"
          description="Sessions appear here once an agent runs against this project and syncs."
        />
      )}

      <ul className="space-y-2" data-testid="session-list">
        {sessions.data?.sessions.map((s) => (
          <li key={s.id}>
            <SessionRow session={s} projectId={id} />
          </li>
        ))}
      </ul>
    </div>
  );
}
