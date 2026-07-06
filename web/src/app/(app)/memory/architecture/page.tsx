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
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { ArrowLeft, Download } from "lucide-react";
import Link from "next/link";
import { useArchitectureReportQuery } from "@/lib/queries";
import type { ArchitectureReport } from "@/lib/api";

function downloadMd(report: ArchitectureReport) {
  const blob = new Blob([report.markdown], { type: "text/markdown" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "architecture-report.md";
  a.click();
  URL.revokeObjectURL(url);
}

export default function ArchitecturePage() {
  const query = useArchitectureReportQuery();

  const report = query.data;

  return (
    <div className="space-y-6 max-w-4xl">
      <header className="flex items-start justify-between gap-3">
        <div>
          <Link
            href="/memory"
            className="text-xs text-muted-foreground hover:text-foreground inline-flex items-center gap-1"
          >
            <ArrowLeft className="size-3" aria-hidden="true" /> Memory
          </Link>
          <h1 className="mt-1 text-2xl font-semibold tracking-tight">Architecture</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Structural analysis of the memory graph: communities, hub nodes, bridges, and cycles.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {report && (
            <Button variant="outline" size="sm" onClick={() => downloadMd(report)}>
              <Download className="mr-1.5 size-4" />
              .md
            </Button>
          )}
          <HelpButton content={HELP["/memory/architecture"]} />
        </div>
      </header>

      {query.isLoading ? (
        <div className="space-y-3">
          <Skeleton className="h-20 w-full" />
          <Skeleton className="h-48 w-full" />
          <Skeleton className="h-32 w-full" />
        </div>
      ) : report ? (
        <>
          {/* Overview */}
          <section className="grid gap-4 md:grid-cols-4">
            <Stat label="Nodes" value={report.file_count} />
            <Stat label="Edges" value={report.edge_count} />
            <Stat label="Communities" value={report.community_count} />
            <Stat label="Isolation" value={`${(report.isolation_ratio * 100).toFixed(1)}%`} />
          </section>

          {/* Language breakdown */}
          <Card>
            <CardHeader>
              <CardTitle>Languages</CardTitle>
              <CardDescription>File extension distribution across nodes.</CardDescription>
            </CardHeader>
            <CardContent>
              {Object.keys(report.language_breakdown).length === 0 ? (
                <p className="text-sm text-muted-foreground">No extensions detected.</p>
              ) : (
                <div className="space-y-1.5">
                  {Object.entries(report.language_breakdown)
                    .sort(([, a], [, b]) => b - a)
                    .map(([ext, count]) => (
                      <div key={ext} className="flex items-center justify-between text-sm">
                        <span className="font-mono">{ext}</span>
                        <Badge variant="outline">{count}</Badge>
                      </div>
                    ))}
                </div>
              )}
            </CardContent>
          </Card>

          {/* God Nodes */}
          {report.god_nodes.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle>God Nodes</CardTitle>
                <CardDescription>
                  Memories with the highest degree centrality (edges &gt; 3). Potential refactoring targets.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b border-line text-left">
                        <th className="pb-2 pr-4 font-medium">Node</th>
                        <th className="pb-2 pr-4 font-medium">Degree</th>
                        <th className="pb-2 font-medium">Kind</th>
                      </tr>
                    </thead>
                    <tbody>
                      {report.god_nodes.map((gn) => (
                        <tr key={gn.name} className="border-b border-line/50">
                          <td className="py-1.5 pr-4 font-mono text-xs">{gn.name}</td>
                          <td className="py-1.5 pr-4">{gn.edge_count}</td>
                          <td className="py-1.5">
                            <Badge variant="outline">{gn.kind}</Badge>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </CardContent>
            </Card>
          )}

          {/* Bridges */}
          {report.bridges.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle>Bridges</CardTitle>
                <CardDescription>
                  Nodes with the highest betweenness centrality — removing them would fragment the graph.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b border-line text-left">
                        <th className="pb-2 pr-4 font-medium">Node</th>
                        <th className="pb-2 pr-4 font-medium">Betweenness</th>
                        <th className="pb-2 font-medium">Kind</th>
                      </tr>
                    </thead>
                    <tbody>
                      {report.bridges.map((b) => (
                        <tr key={b.name} className="border-b border-line/50">
                          <td className="py-1.5 pr-4 font-mono text-xs">{b.name}</td>
                          <td className="py-1.5 pr-4">{b.centrality.toFixed(4)}</td>
                          <td className="py-1.5">
                            <Badge variant="outline">{b.kind}</Badge>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </CardContent>
            </Card>
          )}

          {/* Cycles */}
          <Card>
            <CardHeader>
              <CardTitle>Cycles</CardTitle>
              <CardDescription>
                Circular dependencies in the graph (length &ge; 3).
              </CardDescription>
            </CardHeader>
            <CardContent>
              {report.cycles.length === 0 ? (
                <p className="text-sm text-muted-foreground">No cycles detected.</p>
              ) : (
                <ul className="space-y-1.5">
                  {report.cycles.map((cycle, i) => (
                    <li key={i} className="text-sm font-mono text-muted-foreground">
                      <span className="text-foreground">#{i + 1}</span>{" "}
                      {cycle.join(" → ")}
                    </li>
                  ))}
                </ul>
              )}
            </CardContent>
          </Card>

          {/* Surprising Connections */}
          {report.surprising_connections.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle>Surprising Connections</CardTitle>
                <CardDescription>
                  Edges connecting nodes of different file extensions.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <ul className="space-y-1">
                  {report.surprising_connections.map((conn, i) => (
                    <li key={i} className="text-sm font-mono text-muted-foreground">
                      {conn}
                    </li>
                  ))}
                </ul>
              </CardContent>
            </Card>
          )}
        </>
      ) : null}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="rounded-lg border border-line bg-card p-3">
      <div className="text-sm text-muted-foreground">{label}</div>
      <div className="text-2xl font-semibold">{value}</div>
    </div>
  );
}
