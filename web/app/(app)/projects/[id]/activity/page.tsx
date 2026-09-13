"use client";

import { use, useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import { ArrowDownToLine, Radio } from "lucide-react";
import { api, type ActivityItem } from "@/lib/api";
import {
  ApiErrorState,
  Field,
  NotRecorded,
  humanize,
} from "@/components/control-plane";
import {
  EmptyState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
import { ReferenceChip } from "@/components/reference";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

/**
 * Every event kind and every candidate decision, for the "show everything"
 * control.
 *
 * Listed rather than derived because there is nothing to derive it from in the
 * browser: the server declares its *default* subset on each response, not the
 * full vocabulary. A kind added to the server and not added here is missing
 * from the widened view only — the default view is still the server's own, so
 * this list can never narrow what a reader sees by accident.
 */
const ALL_EVENT_KINDS = [
  "session_opened",
  "session_closed",
  "session_resumed",
  "context_compacting",
  "context_compacted",
  "subagent_started",
  "subagent_completed",
  "tool_started",
  "tool_succeeded",
  "tool_failed",
  "file_read",
  "file_changed",
  "command_executed",
  "test_executed",
  "test_result",
  "research_activity",
  "user_instruction_signal",
  "decision_signal",
  "capture_declined",
  "capture_failed",
  "agent_quiesced",
];
const ALL_DECISIONS = [
  "accepted",
  "reinforced",
  "duplicate",
  "conflicted",
  "refused",
];

export default function ActivityPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);

  // Widening is deliberate and per-view: it starts off on every visit, and the
  // narrow default is the server's declared subset rather than a list this page
  // keeps (FR-882).
  const [everything, setEverything] = useState(false);

  const feed = useInfiniteQuery({
    queryKey: ["activity", id, everything],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) =>
      api.activity(id, {
        cursor: pageParam,
        kinds: everything
          ? [...ALL_EVENT_KINDS, ...ALL_DECISIONS]
          : undefined,
      }),
    getNextPageParam: (last) => last.cursor ?? undefined,
  });

  const items = feed.data?.pages.flatMap((p) => p.items) ?? [];
  const applied = feed.data?.pages[0]?.kinds ?? [];

  return (
    <div>
      <PageHeader
        title="Activity"
        subtitle="What Cairn is receiving from agents, and what consolidation made of it"
      >
        <Button
          variant={everything ? "default" : "outline"}
          size="sm"
          data-testid="activity-show-everything"
          onClick={() => setEverything((on) => !on)}
        >
          <Radio />
          {everything ? "Showing everything" : "Show everything"}
        </Button>
      </PageHeader>

      {/* The subset is stated, not inferred. A reader who could only see what
          arrived would have no way to tell an excluded kind from a kind nothing
          has produced yet (FR-882). */}
      <p className="text-muted-foreground mb-4 text-xs" data-testid="activity-kinds">
        {everything
          ? `Showing all ${applied.length} kinds and decisions.`
          : `Showing the default ${applied.length}: ${applied.map(humanize).join(", ")}.`}
      </p>

      {feed.isLoading && <ListSkeleton rows={4} />}
      {feed.error != null && <ApiErrorState error={feed.error} />}

      {feed.data && items.length === 0 && (
        <EmptyState
          title="Nothing yet"
          description={
            everything
              ? "No events or candidate decisions have been recorded for this project."
              : "Nothing in the default subset has happened yet. Show everything to include routine tool and file activity."
          }
        />
      )}

      <ul className="space-y-2" data-testid="activity-list">
        {items.map((item) => (
          <ActivityRow key={`${item.family}-${item.id}`} item={item} projectId={id} />
        ))}
      </ul>

      {feed.hasNextPage && (
        <div className="mt-4 flex justify-center">
          <Button
            variant="outline"
            size="sm"
            data-testid="activity-more"
            disabled={feed.isFetchingNextPage}
            onClick={() => feed.fetchNextPage()}
          >
            <ArrowDownToLine />
            {feed.isFetchingNextPage ? "Loading…" : "Load more"}
          </Button>
        </div>
      )}
    </div>
  );
}

function ActivityRow({
  item,
  projectId,
}: {
  item: ActivityItem;
  projectId: string;
}) {
  const decision = item.family === "candidate_decision";
  return (
    <li>
      <Card>
        <CardContent>
          <div className="flex flex-wrap items-center gap-2">
            <Badge
              variant={decision ? "default" : "secondary"}
              data-testid="activity-kind"
            >
              {humanize(item.kind)}
            </Badge>
            <Badge variant="outline" data-testid="activity-family">
              {decision ? "consolidation" : "arrival"}
            </Badge>
            {item.agent && (
              <span className="text-muted-foreground text-xs">
                {item.agent}
              </span>
            )}
            <span
              className="text-muted-foreground ml-auto text-xs"
              data-testid="activity-at"
            >
              {formatDate(item.at)}
            </span>
          </div>

          <dl className="mt-3 grid gap-3 sm:grid-cols-2">
            {item.session_id && (
              <Field
                label="Session"
                testId="activity-session"
                value={
                  <code className="font-mono text-xs">
                    {item.session_id.slice(0, 8)}
                  </code>
                }
              />
            )}
            {decision && (
              <Field
                label="Record produced"
                testId="activity-reference"
                value={
                  item.reference ? (
                    <ReferenceChip
                      reference={item.reference}
                      projectId={projectId}
                    />
                  ) : (
                    // Two cases, deliberately indistinguishable: the pass
                    // produced nothing, or it produced a record whose audience
                    // is not this project's members. Naming which would
                    // disclose the record's existence (FR-846a).
                    <NotRecorded what="none visible to you" />
                  )
                }
              />
            )}
            {item.refusal_reason && (
              <Field
                label="Refusal reason"
                testId="activity-refusal"
                value={humanize(item.refusal_reason)}
              />
            )}
          </dl>

          <EventContent content={item.content} />
        </CardContent>
      </Card>
    </li>
  );
}

/**
 * The approved per-kind structure an event carries.
 *
 * Rendered because it is the semantic half: a `file_changed` without the file
 * withholds the only part a reader is actually looking for. There is nothing
 * here to redact — `safe_events` has no column a transcript or a command output
 * could land in, so what arrives is a closed vocabulary of tokens, kinds and
 * repository-relative paths (data-model.md §1.3).
 *
 * Rendered generically rather than per kind because the content is one
 * externally-tagged variant per kind and there are twenty-one of them; a
 * per-kind renderer would go blank for the twenty-second.
 */
function EventContent({ content }: { content: ActivityItem["content"] }) {
  if (content === null) return null;
  // `agent_quiesced` carries nothing, and serializes as the bare variant name.
  if (typeof content === "string") return null;

  const values = Object.values(content);
  const inner =
    values.length === 1 && typeof values[0] === "object" && values[0] !== null
      ? (values[0] as Record<string, unknown>)
      : content;

  const fields = Object.entries(inner).filter(
    ([, v]) => v !== null && typeof v !== "object",
  );
  if (fields.length === 0) return null;

  return (
    <dl
      className="text-muted-foreground mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs"
      data-testid="activity-content"
    >
      {fields.map(([key, value]) => (
        <div key={key} className="flex gap-1">
          <dt>{humanize(key)}:</dt>
          <dd className="text-foreground font-mono break-all">
            {String(value)}
          </dd>
        </div>
      ))}
    </dl>
  );
}
