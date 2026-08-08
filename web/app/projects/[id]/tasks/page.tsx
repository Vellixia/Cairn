"use client";

import { use, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Badge, Card, Empty, ErrorNote, PageHeader, ProjectNav } from "@/components/ui";

const STATUSES = ["all", "todo", "in_progress", "done", "blocked"] as const;

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
    <main>
      <PageHeader title="Tasks" />
      <ProjectNav id={id} active="tasks" />

      <div className="mb-4 flex gap-1">
        {STATUSES.map((s) => (
          <button
            key={s}
            data-testid={`filter-${s}`}
            onClick={() => setStatus(s)}
            className={`rounded border px-2 py-1 text-xs ${
              status === s
                ? "border-neutral-900 dark:border-neutral-100"
                : "border-neutral-300 text-neutral-500 dark:border-neutral-700"
            }`}
          >
            {s.replace("_", " ")}
          </button>
        ))}
      </div>

      {tasks.error != null && <ErrorNote error={tasks.error} />}
      {tasks.data?.tasks.length === 0 && <Empty>No tasks with that status.</Empty>}
      <ul className="space-y-2" data-testid="task-list">
        {tasks.data?.tasks.map((t) => (
          <li key={t.id}>
            <Card>
              <div className="flex items-start justify-between gap-3">
                <div>
                  <div className="font-medium">{t.title}</div>
                  <p className="mt-1 text-sm text-neutral-500">{t.goal}</p>
                </div>
                <Badge>{t.status}</Badge>
              </div>
              {t.acceptance_criteria.length > 0 && (
                <ul className="mt-3 list-inside list-disc text-sm text-neutral-600 dark:text-neutral-400">
                  {t.acceptance_criteria.map((c, i) => (
                    <li key={i}>{c}</li>
                  ))}
                </ul>
              )}
            </Card>
          </li>
        ))}
      </ul>
    </main>
  );
}
