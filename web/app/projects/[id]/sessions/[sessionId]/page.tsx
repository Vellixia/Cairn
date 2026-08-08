"use client";

import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Badge, Card, Empty, ErrorNote, PageHeader, ProjectNav } from "@/components/ui";

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
    <main>
      <PageHeader title="Handoff" subtitle={`Session ${sessionId.slice(0, 8)}`} />
      <ProjectNav id={id} active="sessions" />

      {handoff.error != null && <ErrorNote error={handoff.error} />}
      {handoff.isLoading && <Empty>Loading…</Empty>}

      {handoff.data && (
        <div className="space-y-4" data-testid="handoff">
          <Card>
            <Badge>{handoff.data.handoff.trigger}</Badge>
            <h2 className="mt-2 font-medium">{handoff.data.handoff.goal}</h2>
            <p className="mt-1 text-sm text-neutral-500">
              {handoff.data.handoff.progress}
            </p>
            <p className="mt-3 text-sm">
              <span className="font-medium">Next step:</span>{" "}
              <span data-testid="next-step">{handoff.data.handoff.next_step}</span>
            </p>
          </Card>

          <List title="Remaining work" items={handoff.data.handoff.remaining_work} />
          <List title="Completed" items={handoff.data.handoff.completed_work} />
          <List title="Changed files" items={handoff.data.handoff.changed_files} />
          <List title="Decisions" items={handoff.data.handoff.decisions} />
          <List title="Failures" items={handoff.data.handoff.failures} />

          {handoff.data.handoff.tests_executed.length > 0 && (
            <Card>
              <h3 className="mb-2 text-sm font-medium uppercase tracking-wide text-neutral-500">
                Tests executed
              </h3>
              <ul className="space-y-1 text-sm">
                {handoff.data.handoff.tests_executed.map((t, i) => (
                  <li key={i}>
                    <code>{t.command}</code>
                    <span className="ml-2 text-neutral-500">{t.outcome}</span>
                  </li>
                ))}
              </ul>
            </Card>
          )}

          <Card>
            <h3 className="mb-2 text-sm font-medium uppercase tracking-wide text-neutral-500">
              Repository
            </h3>
            <p className="text-sm">
              <code>{handoff.data.handoff.repository_state.branch}</code> @{" "}
              <code>
                {handoff.data.handoff.repository_state.commit_sha?.slice(0, 8) ??
                  "none"}
              </code>
            </p>
          </Card>

          {handoff.data.handoff.agent_note && (
            <Card>
              <h3 className="mb-2 text-sm font-medium uppercase tracking-wide text-neutral-500">
                Agent note (unverified)
              </h3>
              <p className="text-sm">{handoff.data.handoff.agent_note}</p>
            </Card>
          )}

          <p className="text-xs text-neutral-500">
            {handoff.data.handoff.evidence.evidence_count} supporting
            observation(s). Evidence content stays on the machine that captured
            it.
          </p>
        </div>
      )}
    </main>
  );
}

function List({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <Card>
      <h3 className="mb-2 text-sm font-medium uppercase tracking-wide text-neutral-500">
        {title}
      </h3>
      <ul className="list-inside list-disc space-y-1 text-sm">
        {items.map((i, index) => (
          <li key={index}>{i}</li>
        ))}
      </ul>
    </Card>
  );
}
