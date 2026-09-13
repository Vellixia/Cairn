"use client";

import Link from "next/link";
import { use, useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pin, Search, Trash2, X } from "lucide-react";
import { toast } from "sonner";
import { api, type Memory } from "@/lib/api";
import { ConfirmButton } from "@/components/confirm-button";
import {
  ApiErrorState,
  NotRecorded,
  TruncationNotice,
  humanize,
} from "@/components/control-plane";
import { EmptyState, ListSkeleton, PageHeader } from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const SCOPES = ["project", "branch", "task", "session"];
const TYPES = ["fact", "decision", "convention", "failure", "procedure"];
/** The lifecycle states a project memory can be read in. */
const STATES = ["active", "superseded", "stale", "conflicted"];
/** Base UI's Select has no empty-string value, so "any" is a real option. */
const ANY = "any";

/**
 * The page size, sent explicitly so the notice below can be honest.
 *
 * The server clamps to a hundred and reports back the bound it applied; asking
 * for exactly what the route's own default is keeps the two the same number and
 * makes "you are seeing the first N" a statement about what happened rather
 * than an assumption (FR-895).
 */
const PAGE = 25;

export default function MemoryPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const queryClient = useQueryClient();
  const [q, setQ] = useState("");
  const [scope, setScope] = useState(ANY);
  const [type, setType] = useState(ANY);
  // Not `ANY`: the route filters on exactly one state and defaults to `active`,
  // so there is no "any state" answer it can give. The filter names which one.
  const [state, setState] = useState("active");

  // Typing used to fire one request per keystroke. The input stays instant;
  // only the query waits for a pause.
  const [debouncedQ, setDebouncedQ] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQ(q), 250);
    return () => clearTimeout(timer);
  }, [q]);

  const memories = useQuery({
    queryKey: ["memories", id, debouncedQ, scope, type, state],
    queryFn: () =>
      api.memories(id, {
        q: debouncedQ || undefined,
        scope: scope === ANY ? undefined : scope,
        type: type === ANY ? undefined : type,
        state,
        limit: PAGE,
      }),
  });

  const remove = useMutation({
    mutationFn: (memoryId: string) => api.deleteMemory(memoryId),
    onSuccess: () => {
      toast.success("Memory deleted");
      queryClient.invalidateQueries({ queryKey: ["memories", id] });
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const filtered =
    scope !== ANY || type !== ANY || state !== "active" || debouncedQ !== "";
  const count = memories.data?.memories.length ?? 0;

  return (
    <div>
      <PageHeader
        title="Memory"
        subtitle="Ranked scope-first: task, then branch, then project"
      />

      <div className="mb-3 flex flex-wrap gap-2">
        <div className="relative min-w-56 flex-1">
          <Search className="text-muted-foreground pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2" />
          <Input
            data-testid="memory-search"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Search memory…"
            className="px-9"
          />
          {q && (
            <button
              type="button"
              aria-label="Clear search"
              onClick={() => setQ("")}
              className="text-muted-foreground hover:text-foreground absolute top-1/2 right-2 -translate-y-1/2 rounded p-1"
            >
              <X className="size-3.5" />
            </button>
          )}
        </div>
        <Select value={scope} onValueChange={(v) => setScope(v ?? ANY)}>
          <SelectTrigger
            className="w-36"
            data-testid="scope-filter"
            aria-label="Filter by scope"
          >
            {/* Base UI renders the raw value, which would leave both filters
                reading "any" with no way to tell them apart. */}
            <SelectValue>
              {(v: string) => (v === ANY ? "any scope" : v)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ANY}>any scope</SelectItem>
            {SCOPES.map((s) => (
              <SelectItem key={s} value={s}>
                {s}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={type} onValueChange={(v) => setType(v ?? ANY)}>
          <SelectTrigger
            className="w-36"
            data-testid="type-filter"
            aria-label="Filter by type"
          >
            <SelectValue>
              {(v: string) => (v === ANY ? "any type" : v)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={ANY}>any type</SelectItem>
            {TYPES.map((t) => (
              <SelectItem key={t} value={t}>
                {t}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={state} onValueChange={(v) => setState(v ?? "active")}>
          <SelectTrigger
            className="w-36"
            data-testid="state-filter"
            aria-label="Filter by state"
          >
            <SelectValue>{(v: string) => `${v} only`}</SelectValue>
          </SelectTrigger>
          <SelectContent>
            {STATES.map((s) => (
              <SelectItem key={s} value={s}>
                {s} only
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div
        className="text-muted-foreground mb-4 flex min-h-6 items-center gap-3 text-xs"
        aria-live="polite"
      >
        {memories.data && (
          <span>
            {count} {count === 1 ? "memory" : "memories"}
            {filtered && " matching"}
          </span>
        )}
        {filtered && (
          <Button
            variant="link"
            size="xs"
            className="h-auto p-0"
            onClick={() => {
              setQ("");
              setScope(ANY);
              setType(ANY);
              setState("active");
            }}
          >
            Clear filters
          </Button>
        )}
      </div>

      {memories.isLoading && <ListSkeleton />}
      {memories.error != null && <ApiErrorState error={memories.error} />}
      {memories.data && count === 0 && (
        <EmptyState
          title={filtered ? "No memory matches" : "No memory yet"}
          description={
            filtered
              ? "Try a broader search, or clear the scope, type and state filters."
              : "Memories appear here once an agent records one and syncs."
          }
        />
      )}

      <ul className="space-y-2" data-testid="memory-list">
        {memories.data?.memories.map((m) => (
          <MemoryRow
            key={m.id}
            projectId={id}
            memory={m}
            onDelete={() => remove.mutate(m.id)}
            deleting={remove.isPending}
          />
        ))}
      </ul>

      {memories.data && (
        <TruncationNotice
          shown={count}
          limit={memories.data.limit}
          refine="Narrow the search or the filters to see the rest."
        />
      )}
    </div>
  );
}

function MemoryRow({
  projectId,
  memory,
  onDelete,
  deleting,
}: {
  projectId: string;
  memory: Memory;
  onDelete: () => void;
  deleting: boolean;
}) {
  return (
    <li>
      <Card>
        <CardContent>
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0 flex-1">
              <Link
                href={`/projects/${projectId}/memory/${memory.id}`}
                className="hover:underline"
                data-testid="memory-link"
              >
                <p data-testid="memory-content" className="text-sm">
                  {memory.content}
                </p>
              </Link>
              <div className="text-muted-foreground mt-2 flex flex-wrap items-center gap-2 text-xs">
                <Badge variant="secondary">{memory.type}</Badge>
                <Badge variant="outline">{memory.scope}</Badge>
                {memory.state !== "active" && (
                  <Badge variant="outline">{memory.state}</Badge>
                )}
                <Badge variant="outline" data-testid="memory-importance">
                  {memory.importance}
                </Badge>
                {memory.pinned && (
                  <Badge variant="outline">
                    <Pin /> pinned
                  </Badge>
                )}
                <span data-testid="memory-verification">
                  {/* On the list route this is a bare state string; the detail
                      route replaces the key with an object. Null is a record
                      whose verification was never established, which is not the
                      same as one that failed. */}
                  {memory.verification ? (
                    humanize(memory.verification)
                  ) : (
                    <NotRecorded what="verification not recorded" />
                  )}
                </span>
                <span data-testid="memory-origin">
                  {memory.origin_kind ? (
                    humanize(memory.origin_kind)
                  ) : (
                    <NotRecorded what="origin not recorded" />
                  )}
                </span>
                <span data-testid="memory-relations">
                  {memory.relation_count} relation
                  {memory.relation_count === 1 ? "" : "s"}
                </span>
                <span data-testid="memory-reinforcements">
                  {memory.reinforcement_count} reinforcement
                  {memory.reinforcement_count === 1 ? "" : "s"}
                </span>
                <span>
                  from session{" "}
                  <code data-testid="provenance-session" className="font-mono">
                    {memory.provenance.session_id.slice(0, 8)}
                  </code>
                </span>
                <span data-testid="evidence-count">
                  {memory.provenance.evidence_count} evidence
                </span>
              </div>
              {memory.provenance.evidence_count > 0 && (
                <p className="text-muted-foreground mt-1 text-xs">
                  Evidence content is local to the machine that captured it.
                </p>
              )}
            </div>
            <ConfirmButton
              ariaLabel="Delete memory"
              testId={`delete-${memory.id}`}
              disabled={deleting}
              title="Delete this memory?"
              description="It stops being shared with everyone on this project. Agents that already recalled it keep their local copy."
              confirmLabel="Delete"
              onConfirm={onDelete}
            >
              <Trash2 className="text-destructive size-4" />
            </ConfirmButton>
          </div>
        </CardContent>
      </Card>
    </li>
  );
}
