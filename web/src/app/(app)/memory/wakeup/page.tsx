"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
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

function confidenceColor(c: number) {
  if (c >= 0.8) return "bg-emerald-500";
  if (c >= 0.5) return "bg-amber-500";
  return "bg-destructive";
}

export default function WakeupPage() {
  const memories = useWakeupQuery(50);

  return (
    <div className="space-y-6 max-w-3xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Wakeup</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            The "first thing the agent reads" — high-importance, recently-reinforced
          decisions and tasks. What every new session starts with.
          </p>
        </div>
        <HelpButton content={HELP["/memory/wakeup"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Memories</CardTitle>
          <CardDescription>
            Sorted by importance, confidence, and access count.
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
                    <ItemDescription className="flex items-center gap-2 flex-wrap">
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {m.kind}
                      </Badge>
                      {m.tier} · importance {m.importance.toFixed(2)} · accessed{" "}
                      {m.access_count}×
                      <span className="inline-block h-1.5 w-16 overflow-hidden rounded bg-muted">
                        <span
                          className={`block h-full ${confidenceColor(m.confidence)}`}
                          style={{ width: `${Math.max(0, Math.min(100, m.confidence * 100))}%` }}
                          aria-label={`confidence ${(m.confidence * 100).toFixed(0)}%`}
                        />
                      </span>
                      <span className="font-mono text-[10px] text-muted-foreground">
                        conf {m.confidence.toFixed(2)}
                      </span>
                      {m.pinned && (
                        <Badge variant="secondary" className="text-[10px]">
                          pinned
                        </Badge>
                      )}
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
