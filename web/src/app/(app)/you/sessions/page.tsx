"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { useQuery } from "@tanstack/react-query";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { getJSON } from "@/lib/api";

interface Session {
  id: string;
  project_hash: string;
  started_at: string;
  ended_at: string | null;
  tasks: Array<{ id: string; title: string; progress: string }>;
  findings: Array<{ text: string; source_file?: string; confidence: number }>;
  decisions: Array<{ text: string; rationale: string; confidence: number }>;
  touched_files: Array<{ path: string; mode: string }>;
  next_steps: string[];
  memory_ids: string[];
}

export default function SessionsPage() {
  const sessions = useQuery({
    queryKey: ["sessions-list"],
    queryFn: () => getJSON<Session[]>(`/api/sessions`),
  });

  return (
    <div className="space-y-6 max-w-3xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Sessions</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Every session you logged with `cairn session start`. Open one to
          see its CCP block + tasks + decisions.
          </p>
        </div>
        <HelpButton content={HELP["/you/sessions"]} />
      </header>
      {sessions.isLoading ? (
        <Skeleton className="h-72 w-full" />
      ) : sessions.data && sessions.data.length > 0 ? (
        <div className="space-y-2">

          {sessions.data.map((s) => (
            <Card key={s.id}>
              <CardHeader>
                <CardTitle className="text-base">
                  <a
                    href={`/dashboard/sessions/${encodeURIComponent(s.id)}`}
                    className="underline"
                  >
                    {s.id}
                  </a>
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-xs text-muted-foreground">
                  {s.project_hash} --- started {s.started_at}
                </p>
                <p className="text-sm mt-1">
                  {s.tasks.length} tasks, {s.decisions.length} decisions,{" "}
                  {s.findings.length} findings
                </p>
              </CardContent>
            </Card>
          ))}
        </div>
      ) : (
        <Card className="p-6 text-sm text-muted-foreground">
          No sessions yet. Start one with{" "}
          <code className="font-mono">cairn session start</code>.
        </Card>
      )}
    </div>
  );
}
