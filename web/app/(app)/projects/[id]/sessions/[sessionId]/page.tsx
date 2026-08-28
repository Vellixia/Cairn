"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, GitCommitHorizontal } from "lucide-react";
import { api } from "@/lib/api";
import { ErrorState, ListSkeleton, PageHeader } from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

export default function HandoffPage({
  params,
}: {
  params: Promise<{ id: string; sessionId: string }>;
}) {
  const { id, sessionId } = use(params);
  const handoff = useQuery({
    queryKey: ["handoff", sessionId],
    queryFn: () => api.handoff(sessionId),
  });

  return (
    <div>
      <Button
        variant="ghost"
        size="sm"
        className="mb-3 -ml-2"
        render={<Link href={`/projects/${id}/sessions`} />}
      >
        <ArrowLeft />
        All sessions
      </Button>

      <PageHeader
        title="Handoff"
        subtitle={`Session ${sessionId.slice(0, 8)}`}
      />

      {handoff.isLoading && <ListSkeleton rows={3} />}
      {handoff.error != null && <ErrorState error={handoff.error} />}

      {handoff.data && (
        <div className="space-y-4" data-testid="handoff">
          <Card>
            <CardContent>
              <Badge variant="secondary">
                {handoff.data.handoff.trigger.replace("_", " ")}
              </Badge>
              <h2 className="mt-2 font-medium">{handoff.data.handoff.goal}</h2>
              <p className="text-muted-foreground mt-1 text-sm">
                {handoff.data.handoff.progress}
              </p>
              <div className="bg-muted/50 mt-3 rounded-md p-3 text-sm">
                <span className="font-medium">Next step: </span>
                <span data-testid="next-step">
                  {handoff.data.handoff.next_step}
                </span>
              </div>
            </CardContent>
          </Card>

          <div className="grid gap-4 lg:grid-cols-2">
            <List
              title="Remaining work"
              items={handoff.data.handoff.remaining_work}
            />
            <List
              title="Completed"
              items={handoff.data.handoff.completed_work}
            />
            <List
              title="Changed files"
              items={handoff.data.handoff.changed_files}
              mono
            />
            <List title="Decisions" items={handoff.data.handoff.decisions} />
            <List title="Failures" items={handoff.data.handoff.failures} />
          </div>

          {handoff.data.handoff.tests_executed.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle className="text-sm font-medium">
                  Tests executed
                </CardTitle>
              </CardHeader>
              <CardContent>
                <ul className="space-y-1.5 text-sm">
                  {handoff.data.handoff.tests_executed.map((t, i) => (
                    <li key={i} className="flex flex-wrap items-center gap-2">
                      <code className="bg-muted rounded px-1.5 py-0.5 font-mono text-xs">
                        {t.runner}
                      </code>
                      <span className="text-muted-foreground text-xs">
                        {t.outcome}
                      </span>
                    </li>
                  ))}
                </ul>
              </CardContent>
            </Card>
          )}

          <Card>
            <CardHeader>
              <CardTitle className="text-sm font-medium">Repository</CardTitle>
            </CardHeader>
            <CardContent>
              <p className="flex flex-wrap items-center gap-2 text-sm">
                <GitCommitHorizontal className="text-muted-foreground size-4" />
                <code className="font-mono text-xs">
                  {handoff.data.handoff.repository_state.branch}
                </code>
                <span className="text-muted-foreground">@</span>
                <code className="font-mono text-xs">
                  {handoff.data.handoff.repository_state.commit_sha?.slice(
                    0,
                    8,
                  ) ?? "none"}
                </code>
              </p>
              <p className="text-muted-foreground mt-2 text-xs">
                {handoff.data.handoff.repository_state.staged} staged ·{" "}
                {handoff.data.handoff.repository_state.unstaged} unstaged ·{" "}
                {handoff.data.handoff.repository_state.untracked} untracked
              </p>
            </CardContent>
          </Card>

          {handoff.data.handoff.agent_note && (
            <Card>
              <CardHeader>
                <CardTitle className="text-sm font-medium">
                  Agent note
                  <Badge variant="outline" className="ml-2">
                    unverified
                  </Badge>
                </CardTitle>
              </CardHeader>
              <CardContent>
                <p className="text-sm">{handoff.data.handoff.agent_note}</p>
              </CardContent>
            </Card>
          )}

          <p className="text-muted-foreground text-xs">
            {handoff.data.handoff.evidence.evidence_count} supporting
            observation(s). Evidence content stays on the machine that captured
            it.
          </p>
        </div>
      )}
    </div>
  );
}

function List({
  title,
  items,
  mono = false,
}: {
  title: string;
  items: string[];
  mono?: boolean;
}) {
  if (items.length === 0) return null;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">{title}</CardTitle>
      </CardHeader>
      <CardContent>
        <ul className="space-y-1 text-sm">
          {items.map((item, index) => (
            <li key={index} className="flex gap-2">
              <span aria-hidden className="text-muted-foreground select-none">
                •
              </span>
              <span className={mono ? "font-mono text-xs break-all" : ""}>
                {item}
              </span>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}
