"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemTitle,
} from "@/components/ui/item";
import { getJSON } from "@/lib/api";

export default function ProfilePage() {
  const prefs = useQuery({
    queryKey: ["profile", "list"],
    queryFn: () => getJSON<MemoryLite[]>("/api/profile"),
  });

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Profile</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Standing preferences that every Cairn-backed agent honors. Use{" "}
          <code>cairn prefer</code> or the <code>prefer</code> MCP tool to add
          or update them.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Active preferences</CardTitle>
          <CardDescription>
            {prefs.data
              ? `${prefs.data.length} stored . sorted newest first`
              : "Loading..."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {prefs.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : prefs.data && prefs.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No preferences yet. Use <code>cairn prefer</code> to add one.
            </p>
          ) : (
            <ul className="space-y-2">
              {prefs.data?.map((p) => (
                <Item key={p.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="line-clamp-2">{p.content}</ItemTitle>
                    <ItemDescription className="flex items-center gap-2">
                      <Badge
                        variant="outline"
                        className="font-mono text-[10px] uppercase tracking-wider"
                      >
                        {p.kind}
                      </Badge>
                      <ConfidenceBar value={p.confidence} />
                      <span className="font-mono text-[10px] text-muted-foreground">
                        conf {p.confidence.toFixed(2)}
                      </span>
                      {p.pinned && (
                        <Badge variant="secondary" className="text-[10px]">
                          pinned
                        </Badge>
                      )}
                      {p.suspicious && (
                        <Badge variant="destructive" className="text-[10px]">
                          suspicious
                        </Badge>
                      )}
                    </ItemDescription>
                  </ItemContent>
                </Item>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <p className="text-[11px] text-muted-foreground">
        Use the{" "}
        <Link href="/memory" className="underline">
          memory browser&apos;s Wakeup-order toggle
        </Link>{" "}
        to inspect how preferences flow into session bootstrap.
      </p>
    </div>
  );
}

interface MemoryLite {
  id: string;
  kind: string;
  tier: string;
  content: string;
  confidence: number;
  pinned: boolean;
  suspicious: boolean;
  created_at: string;
}

function ConfidenceBar({ value }: { value: number }) {
  const pct = Math.max(0, Math.min(100, value * 100));
  const color =
    pct >= 80
      ? "bg-emerald-500"
      : pct >= 50
        ? "bg-amber-500"
        : "bg-destructive";
  return (
    <span className="inline-block h-1.5 w-16 overflow-hidden rounded bg-muted">
      <span
        className={`block h-full ${color}`}
        style={{ width: `${pct}%` }}
        aria-label={`confidence ${pct.toFixed(0)}%`}
      />
    </span>
  );
}
