"use client";

import { useState } from "react";
import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { ArrowDownToLine, Check, Archive } from "lucide-react";
import { toast } from "sonner";
import { api, type TeamKnowledge } from "@/lib/api";
import { ConfirmButton } from "@/components/confirm-button";
import { ApiErrorState, humanize } from "@/components/control-plane";
import {
  EmptyState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";

/**
 * Review, ratify and retire team guidance (FR-889, FR-889a).
 *
 * **There is no authoring affordance here, and that is deliberate.** An agent
 * may propose team knowledge through the sync path; only a human administrator
 * may make it authoritative. Adding a "new proposal" or an "edit" control to
 * this screen would put authorship and ratification in the same pair of hands
 * at the same moment, which is the separation the lifecycle exists to keep.
 */
export default function TeamPage() {
  const queryClient = useQueryClient();
  const [acting, setActing] = useState<string | null>(null);

  // Role decides which buttons are worth offering, never who may act. Both
  // transitions are `AdminUser`-gated on the server and refuse a member before
  // their handlers run; this only avoids showing a door that will not open.
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.me() });
  const isAdmin = me.data?.role === "admin";

  const feed = useInfiniteQuery({
    queryKey: ["team-knowledge"],
    initialPageParam: undefined as string | undefined,
    queryFn: ({ pageParam }) => api.teamKnowledge({ cursor: pageParam }),
    getNextPageParam: (last) => last.cursor ?? undefined,
  });

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["team-knowledge"] });

  /**
   * Both transitions post an id and nothing else.
   *
   * The server's handler is a single `UPDATE ... WHERE state = '...'
   * RETURNING`, so two administrators acting at once race inside PostgreSQL and
   * exactly one wins; the loser is told what the state actually is. A client
   * that read the row, decided it was ratifiable, and then posted an
   * unconditional change would move that race into the browser where it cannot
   * be resolved — and would make "un-retire" expressible, because a stale read
   * could be replayed against changed state (FR-889a).
   */
  const ratify = useMutation({
    mutationFn: (id: string) => api.ratifyTeam(id),
    onMutate: (id: string) => setActing(id),
    onSuccess: () => toast.success("Ratified"),
    onError: (err: Error) => toast.error(err.message),
    onSettled: () => {
      setActing(null);
      invalidate();
    },
  });

  const retire = useMutation({
    mutationFn: (id: string) => api.retireTeam(id),
    onMutate: (id: string) => setActing(id),
    onSuccess: () => toast.success("Retired"),
    onError: (err: Error) => toast.error(err.message),
    onSettled: () => {
      setActing(null);
      invalidate();
    },
  });

  const items = feed.data?.pages.flatMap((p) => p.items) ?? [];
  const proposals = items.filter((k) => k.state === "proposed");

  return (
    <div>
      <PageHeader
        title="Team knowledge"
        subtitle="Server-wide guidance: what has been proposed, what is authoritative, what has been retired"
      />

      {!isAdmin && me.data && (
        <p className="text-muted-foreground mb-4 text-sm" data-testid="team-readonly">
          Ratifying and retiring are administrator actions. You can read what is
          visible to you here; proposals stay visible to their author and to
          administrators until they are ratified.
        </p>
      )}

      {isAdmin && (
        <p className="text-muted-foreground mb-4 text-sm" data-testid="team-queue">
          {proposals.length} proposal{proposals.length === 1 ? "" : "s"} awaiting
          review on this page.
        </p>
      )}

      {feed.isLoading && <ListSkeleton rows={3} />}
      {feed.error != null && <ApiErrorState error={feed.error} />}

      {feed.data && items.length === 0 && (
        <EmptyState
          title="No team knowledge"
          description="Proposals are created from the CLI with cairn team propose. This screen reviews them."
        />
      )}

      <ul className="space-y-2" data-testid="team-list">
        {items.map((k) => (
          <TeamRow
            key={k.id}
            entry={k}
            isAdmin={isAdmin}
            busy={acting === k.id}
            onRatify={() => ratify.mutate(k.id)}
            onRetire={() => retire.mutate(k.id)}
          />
        ))}
      </ul>

      {feed.hasNextPage && (
        <div className="mt-4 flex justify-center">
          <Button
            variant="outline"
            size="sm"
            data-testid="team-more"
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

function TeamRow({
  entry,
  isAdmin,
  busy,
  onRatify,
  onRetire,
}: {
  entry: TeamKnowledge;
  isAdmin: boolean;
  busy: boolean;
  onRatify: () => void;
  onRetire: () => void;
}) {
  return (
    <li>
      <Card data-testid="team-row" data-entry-id={entry.id} data-state={entry.state}>
        <CardContent>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="min-w-0 flex-1">
              <p className="text-sm" data-testid="team-content">
                {entry.content}
              </p>
              <div className="text-muted-foreground mt-2 flex flex-wrap items-center gap-2 text-xs">
                <Badge
                  variant={
                    entry.state === "authoritative" ? "default" : "outline"
                  }
                  data-testid="team-state"
                >
                  {entry.state}
                </Badge>
                <Badge variant="secondary">{entry.knowledge_type}</Badge>
                {entry.topic_key && (
                  <code className="font-mono">
                    {entry.topic_key}
                    {entry.value_key ? ` = ${entry.value_key}` : ""}
                  </code>
                )}
                <span>proposed {formatDate(entry.created_at)}</span>
                {entry.ratified_at && (
                  <span data-testid="team-ratified">
                    ratified {formatDate(entry.ratified_at)}
                  </span>
                )}
                {/* Who as well as when. Retirement withdraws guidance from every
                    account on the server, so it is the transition most worth
                    attributing (FR-457). */}
                {entry.retired_at && (
                  <span data-testid="team-retired">
                    retired {formatDate(entry.retired_at)}
                    {entry.retired_by_user_id
                      ? ` by ${entry.retired_by_user_id.slice(0, 8)}`
                      : ""}
                  </span>
                )}
              </div>
            </div>

            {isAdmin && (
              <div className="flex shrink-0 gap-2">
                {/* Each button is offered only from the state its transition
                    accepts. That is a convenience, not the guard: the server's
                    `WHERE state = ...` is what actually refuses a second
                    ratification, and it still refuses one if this button is
                    somehow clicked twice. */}
                {entry.state === "proposed" && (
                  <ConfirmButton
                    ariaLabel="Ratify this proposal"
                    testId="team-ratify"
                    disabled={busy}
                    title="Make this authoritative?"
                    description="Every account on this server starts receiving it as guidance. Ratification is recorded against your account."
                    confirmLabel="Ratify"
                    onConfirm={onRatify}
                  >
                    <Check className="size-4" />
                  </ConfirmButton>
                )}
                {entry.state === "authoritative" && (
                  <ConfirmButton
                    ariaLabel="Retire this entry"
                    testId="team-retire"
                    disabled={busy}
                    title="Retire this guidance?"
                    description="It stops reaching every account on this server, and there is no route back: restoring it means a new proposal, ratified again."
                    confirmLabel="Retire"
                    onConfirm={onRetire}
                  >
                    <Archive className="text-destructive size-4" />
                  </ConfirmButton>
                )}
              </div>
            )}
          </div>
        </CardContent>
      </Card>
    </li>
  );
}
