"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import dynamic from "next/dynamic";
import { useQuery } from "@tanstack/react-query";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { useCrystallizeMutation } from "@/lib/queries";
import { getJSON } from "@/lib/api";

// react-force-graph is a client-only library (uses canvas / d3-force); load with `ssr: false`
// to avoid "document is not defined" during the static export build.
const ForceGraph = dynamic(
  () => import("@/components/MemoryForceGraph").then((m) => m.MemoryForceGraph),
  { ssr: false, loading: () => <Skeleton className="h-[480px] w-full" /> },
);

interface GraphResponse {
  nodes: Array<{
    id: string;
    kind: string;
    tier: string;
    content_preview: string;
    confidence: number;
    pinned: boolean;
    importance: number;
  }>;
  edges: Array<{
    source: string;
    target: string;
    kind: string;
  }>;
}

export default function MemoryGraphPage() {
  const graph = useQuery({
    queryKey: ["memory", "graph"],
    queryFn: () => getJSON<GraphResponse>("/api/memory/graph"),
    refetchInterval: 10_000,
  });
  const crystallize = useCrystallizeMutation();

  const stats = graph.data
    ? {
        nodes: graph.data.nodes.length,
        edges: graph.data.edges.length,
        pinned: graph.data.nodes.filter((n) => n.pinned).length,
        crystals: graph.data.nodes.filter((n) => n.tier === "semantic").length,
      }
    : null;

  return (
    <div className="space-y-6">

      <header className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Memory graph</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            The provenance graph: every memory as a node, every{" "}
            <code>derived_from</code> / <code>contradicts</code> /{" "}
            <code>supersedes</code> / <code>applies_to</code> edge as a link. Node size is
            importance, color is tier.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          disabled={crystallize.isPending}
          onClick={() => crystallize.mutate({})}
        >
          {crystallize.isPending ? "Crystallizing…" : "Crystallize working"}
        </Button>
      </header>

      <section className="grid gap-4 md:grid-cols-4">
        <Stat label="Nodes" value={stats?.nodes ?? "…"} />
        <Stat label="Edges" value={stats?.edges ?? "…"} />
        <Stat label="Pinned" value={stats?.pinned ?? "…"} />
        <Stat label="Crystals" value={stats?.crystals ?? "…"} />
      </section>

      <Card>
        <CardHeader>
          <CardTitle>Graph</CardTitle>
          <CardDescription>
            {graph.isLoading
              ? "Loading…"
              : graph.data
                ? `${graph.data.nodes.length} memories · ${graph.data.edges.length} edges`
                : "—"}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {graph.isLoading ? (
            <Skeleton className="h-[480px] w-full" />
          ) : graph.data && graph.data.nodes.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No memories yet. Remember something to start the graph.
            </p>
          ) : graph.data ? (
            <ForceGraph data={graph.data} />
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string | number }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground font-normal">
          {label}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-semibold font-mono">{value}</div>
      </CardContent>
    </Card>
  );
}