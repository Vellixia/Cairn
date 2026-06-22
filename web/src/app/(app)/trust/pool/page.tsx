"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Item, ItemContent, ItemTitle, ItemDescription } from "@/components/ui/item";
import { usePoolQuery, usePublishPoolMutation } from "@/lib/queries";

const SENSITIVITY: Record<
  "shareable" | "needs_review" | "private",
  "default" | "secondary" | "destructive"
> = {
  shareable: "default",
  needs_review: "secondary",
  private: "destructive",
};

export default function PoolPage() {
  const pool = usePoolQuery();
  const publish = usePublishPoolMutation();

  return (
    <div className="space-y-6 max-w-3xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Pool</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            The collective, sanitized knowledge this server shares with other
          Cairn servers.
          </p>
        </div>
        <HelpButton content={HELP["/trust/pool"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Publish</CardTitle>
          <CardDescription>
            Snapshot your shareable memories into the local pool, then federate.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex items-center gap-3">

            <Button onClick={() => publish.mutate()} disabled={publish.isPending}>
              {publish.isPending
                ? "Publishing…"
                : "Publish my shareable memories"}
            </Button>
            {pool.data && (
              <span className="text-sm text-muted-foreground">
                {pool.data.count} in pool
              </span>
            )}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>In pool</CardTitle>
        </CardHeader>
        <CardContent>
          {pool.isLoading ? (
            <div className="space-y-2">

              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : pool.data && pool.data.memories.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              Empty pool. Publish your shareable memories to start contributing.
            </p>
          ) : pool.data ? (
            <ul className="space-y-2">
              {pool.data.memories.map((m, i) => (
                <Item key={i} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="line-clamp-2">{m.content}</ItemTitle>
                    <ItemDescription className="flex items-center gap-2">
                      <Badge
                        variant={SENSITIVITY[m.sensitivity]}
                        className="capitalize"
                      >
                        {m.sensitivity}
                      </Badge>
                      <span className="text-muted-foreground">{m.kind}</span>
                      {m.redactions > 0 && (
                        <span className="text-muted-foreground">
                          · {m.redactions} redaction
                          {m.redactions === 1 ? "" : "s"}
                        </span>
                      )}
                    </ItemDescription>
                  </ItemContent>
                </Item>
              ))}
            </ul>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Federate across machines</CardTitle>
          <CardDescription>
            Pull and push the pool with another Cairn instance.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <pre className="rounded-md border border-line bg-secondary p-3 font-mono text-xs overflow-x-auto">{`cairn contribute --server ${typeof window !== "undefined" ? window.location.origin : "<server>"}
cairn pull --server ${typeof window !== "undefined" ? window.location.origin : "<server>"}`}</pre>
        </CardContent>
      </Card>
    </div>
  );
}
