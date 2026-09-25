"use client";

import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import {
  EmptyState,
  ErrorState,
  ListSkeleton,
  PageHeader,
} from "@/components/page";
import { SessionRow } from "@/components/session";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatDate } from "@/components/page";

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
  const replay = useQuery({ queryKey: ["replay", id], queryFn: () => api.replay(id) });

  return (
    <div>
      <PageHeader
        title="Sessions & Replay"
        subtitle="Accepted sessions, newest first. Replay remains read-only."
      />

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
      <Card className="mt-6" data-testid="accepted-event-replay">
        <CardHeader><CardTitle className="text-sm font-medium">Accepted event replay</CardTitle></CardHeader>
        <CardContent>
          {replay.isLoading && <p className="text-muted-foreground text-sm">Loading…</p>}
          {replay.data?.events.length === 0 && <p className="text-muted-foreground text-sm">No accepted events.</p>}
          <ul className="space-y-1 text-sm">{replay.data?.events.map((event) => <li key={event.event_id}><code className="text-xs">{event.kind}</code> · {formatDate(event.accepted_at)} · session {event.session_id.slice(0, 8)}</li>)}</ul>
        </CardContent>
      </Card>
    </div>
  );
}
