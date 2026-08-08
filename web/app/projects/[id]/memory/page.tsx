"use client";

import { use, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type Memory } from "@/lib/api";
import { Badge, Card, Empty, ErrorNote, PageHeader, ProjectNav } from "@/components/ui";

const SCOPES = ["", "project", "branch", "task", "session"];
const TYPES = ["", "fact", "decision", "convention", "failure", "procedure"];

export default function MemoryPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const queryClient = useQueryClient();
  const [q, setQ] = useState("");
  const [scope, setScope] = useState("");
  const [type, setType] = useState("");

  const memories = useQuery({
    queryKey: ["memories", id, q, scope, type],
    queryFn: () =>
      api.memories(id, {
        q: q || undefined,
        scope: scope || undefined,
        type: type || undefined,
      }),
  });

  const remove = useMutation({
    mutationFn: (memoryId: string) => api.deleteMemory(memoryId),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["memories", id] }),
  });

  return (
    <main>
      <PageHeader
        title="Memory"
        subtitle="Ranked scope-first: task, then branch, then project"
      />
      <ProjectNav id={id} active="memory" />

      <div className="mb-4 flex flex-wrap gap-2">
        <input
          data-testid="memory-search"
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="Search memory…"
          className="flex-1 rounded border border-neutral-300 px-2 py-1.5 text-sm dark:border-neutral-700 dark:bg-neutral-950"
        />
        <select
          data-testid="scope-filter"
          value={scope}
          onChange={(e) => setScope(e.target.value)}
          className="rounded border border-neutral-300 px-2 py-1.5 text-sm dark:border-neutral-700 dark:bg-neutral-950"
        >
          {SCOPES.map((s) => (
            <option key={s} value={s}>
              {s === "" ? "any scope" : s}
            </option>
          ))}
        </select>
        <select
          data-testid="type-filter"
          value={type}
          onChange={(e) => setType(e.target.value)}
          className="rounded border border-neutral-300 px-2 py-1.5 text-sm dark:border-neutral-700 dark:bg-neutral-950"
        >
          {TYPES.map((t) => (
            <option key={t} value={t}>
              {t === "" ? "any type" : t}
            </option>
          ))}
        </select>
      </div>

      {memories.error != null && <ErrorNote error={memories.error} />}
      {memories.data?.memories.length === 0 && (
        <Empty>No memory matches that search.</Empty>
      )}

      <ul className="space-y-2" data-testid="memory-list">
        {memories.data?.memories.map((m) => (
          <MemoryRow
            key={m.id}
            memory={m}
            onDelete={() => remove.mutate(m.id)}
            deleting={remove.isPending}
          />
        ))}
      </ul>
    </main>
  );
}

function MemoryRow({
  memory,
  onDelete,
  deleting,
}: {
  memory: Memory;
  onDelete: () => void;
  deleting: boolean;
}) {
  return (
    <li>
      <Card>
        <div className="flex items-start justify-between gap-3">
          <div className="flex-1">
            <p data-testid="memory-content">{memory.content}</p>
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-neutral-500">
              <Badge>{memory.type}</Badge>
              <Badge>{memory.scope}</Badge>
              <span>
                from session{" "}
                <code data-testid="provenance-session">
                  {memory.provenance.session_id.slice(0, 8)}
                </code>
              </span>
              <span data-testid="evidence-count">
                {memory.provenance.evidence_count} evidence
              </span>
            </div>
            {memory.provenance.evidence_count > 0 && (
              <p className="mt-1 text-xs text-neutral-400">
                Evidence content is local to the machine that captured it.
              </p>
            )}
          </div>
          <button
            data-testid={`delete-${memory.id}`}
            onClick={onDelete}
            disabled={deleting}
            className="rounded border border-red-300 px-2 py-1 text-xs text-red-600 disabled:opacity-50"
          >
            Delete
          </button>
        </div>
      </Card>
    </li>
  );
}
