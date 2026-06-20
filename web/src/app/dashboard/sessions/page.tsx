"use client";

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemTitle,
  ItemDescription,
} from "@/components/ui/item";
import { Input } from "@/components/ui/input";
import { getJSON, postJSON } from "@/lib/api";

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
  const qc = useQueryClient();
  const sessions = useQuery({
    queryKey: ["sessions"],
    queryFn: () => getJSON<Session[]>("/api/sessions"),
    refetchInterval: 5_000,
  });
  const latest = useQuery({
    queryKey: ["sessions", "latest"],
    queryFn: () => getJSON<{ session: Session | null; block: string }>("/api/sessions/latest"),
  });
  const [project, setProject] = useState("default");

  async function startNew() {
    await postJSON<Session>("/api/sessions", {
      project_hash: project.trim() || "default",
    });
    qc.invalidateQueries({ queryKey: ["sessions"] });
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Sessions</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Cross-Session Protocol (CCP) records. The most-recent session is auto-injected at
          the next agent start.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Latest CCP block</CardTitle>
          <CardDescription>
            What the next session will read first.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {latest.isLoading ? (
            <Skeleton className="h-24 w-full" />
          ) : latest.data?.block ? (
            <pre className="max-h-72 overflow-auto rounded-md border border-line bg-secondary p-3 font-mono text-xs leading-relaxed">
              {latest.data.block}
            </pre>
          ) : (
            <p className="text-sm text-muted-foreground">
              No sessions yet. Start one below.
            </p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Start a new session</CardTitle>
          <CardDescription>
            Tags the session with a project hash; the CLI derives this from the working
            directory.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex gap-2">
            <Input
              value={project}
              onChange={(e) => setProject(e.target.value)}
              placeholder="project-hash"
            />
            <Button onClick={startNew}>Start</Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>History</CardTitle>
          <CardDescription>
            {sessions.data ? `${sessions.data.length} session(s)` : "Loading…"}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {sessions.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : sessions.data && sessions.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">No sessions yet.</p>
          ) : (
            <ul className="space-y-2">
              {sessions.data?.map((s) => (
                <Item key={s.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="font-mono text-xs">
                      {s.id.slice(0, 8)}
                    </ItemTitle>
                    <ItemDescription>
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {s.project_hash}
                      </Badge>
                      {new Date(s.started_at).toLocaleString()}
                      {s.ended_at && (
                        <span className="ml-2 text-[10px] text-muted-foreground">
                          closed
                        </span>
                      )}
                      <span className="ml-2 text-[10px] text-muted-foreground">
                        {s.tasks.length}t · {s.findings.length}f · {s.decisions.length}d ·{" "}
                        {s.next_steps.length}n
                      </span>
                    </ItemDescription>
                  </ItemContent>
                  <ItemActions>
                    <Button asChild variant="ghost" size="sm">
                      <a href={`/dashboard/sessions/${s.id}`}>Open</a>
                    </Button>
                  </ItemActions>
                </Item>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}