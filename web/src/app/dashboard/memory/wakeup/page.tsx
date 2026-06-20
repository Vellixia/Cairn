"use client";

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
import { useWakeupQuery } from "@/lib/queries";

export default function WakeupPage() {
  const memories = useWakeupQuery(50);

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Wakeup</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          The "first thing the agent reads" — high-importance, recently-reinforced
          decisions and tasks. What every new session starts with.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Memories</CardTitle>
          <CardDescription>
            Sorted by importance and access count.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {memories.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : memories.data && memories.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">Nothing to wake up to yet.</p>
          ) : memories.data ? (
            <ul className="space-y-2">
              {memories.data.map((m) => (
                <Item key={m.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="line-clamp-2">{m.content}</ItemTitle>
                    <ItemDescription>
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {m.kind}
                      </Badge>
                      {m.tier} · importance {m.importance.toFixed(2)} · accessed{" "}
                      {m.access_count}×
                    </ItemDescription>
                  </ItemContent>
                </Item>
              ))}
            </ul>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}
