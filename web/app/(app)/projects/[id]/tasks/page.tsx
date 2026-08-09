"use client";

import { use, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, type Task } from "@/lib/api";
import {
  EmptyState,
  ErrorState,
  ListSkeleton,
  PageHeader,
} from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

const STATUSES = ["all", "todo", "in_progress", "done", "blocked"] as const;

function statusVariant(status: Task["status"]) {
  switch (status) {
    case "blocked":
      return "destructive" as const;
    case "done":
      return "secondary" as const;
    case "in_progress":
      return "default" as const;
    default:
      return "outline" as const;
  }
}

export default function TasksPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const [status, setStatus] = useState<string>("all");
  const tasks = useQuery({
    queryKey: ["tasks", id, status],
    queryFn: () => api.tasks(id, status === "all" ? undefined : status),
  });

  return (
    <div>
      <PageHeader title="Tasks" subtitle="What agents are working towards" />

      {/* Toggle buttons rather than tabs: these filter a list in place, and a
          tablist whose tabs control no panel misleads a screen reader. */}
      <ToggleGroup
        value={[status]}
        onValueChange={(value) => setStatus(value[0] ?? "all")}
        aria-label="Filter tasks by status"
        className="mb-4"
      >
        {STATUSES.map((s) => (
          <ToggleGroupItem key={s} value={s} data-testid={`filter-${s}`}>
            {s.replace("_", " ")}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>

      {tasks.isLoading && <ListSkeleton />}
      {tasks.error != null && <ErrorState error={tasks.error} />}
      {tasks.data?.tasks.length === 0 && (
        <EmptyState
          title="Nothing here"
          description={
            status === "all"
              ? "No tasks have been synced for this project yet."
              : `No tasks are ${status.replace("_", " ")}.`
          }
        />
      )}

      <ul className="space-y-3" data-testid="task-list">
        {tasks.data?.tasks.map((t) => (
          <li key={t.id}>
            <Card>
              <CardContent>
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="font-medium">{t.title}</div>
                    <p className="text-muted-foreground mt-1 text-sm">
                      {t.goal}
                    </p>
                  </div>
                  <Badge variant={statusVariant(t.status)} className="shrink-0">
                    {t.status.replace("_", " ")}
                  </Badge>
                </div>
                {t.acceptance_criteria.length > 0 && (
                  <ul className="text-muted-foreground mt-3 space-y-1 text-sm">
                    {t.acceptance_criteria.map((c, i) => (
                      <li key={i} className="flex gap-2">
                        <span aria-hidden className="select-none">
                          •
                        </span>
                        <span>{c}</span>
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>
          </li>
        ))}
      </ul>
    </div>
  );
}
