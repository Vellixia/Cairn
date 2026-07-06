"use client";

import Link from "next/link";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import {
  Item,
  ItemContent,
  ItemTitle,
  ItemDescription,
} from "@/components/ui/item";
import {
  useProjectQuery,
  useMemoryByScopeQuery,
  usePromotionLogQuery,
  useDocumentsQuery,
} from "@/lib/queries";
import { displayTitle } from "@/lib/memoryTitle";
import { useUrlId } from "@/lib/useUrlId";

export default function ProjectDetail() {
  const id = useUrlId() ?? "";
  const project = useProjectQuery(id);
  const memories = useMemoryByScopeQuery("project", id, 50);
  const promotionLog = usePromotionLogQuery(200);
  const documents = useDocumentsQuery(id);

  const promotionActivity = (promotionLog.data ?? []).filter((e) => e.old_scope_id === id);

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <p className="text-xs text-muted-foreground">
          <Link href="/projects" className="underline underline-offset-2">
            Projects
          </Link>
        </p>
        {project.isLoading ? (
          <Skeleton className="h-8 w-64 mt-1" />
        ) : project.data ? (
          <>
            <h1 className="text-2xl font-semibold tracking-tight">{project.data.name}</h1>
            <p className="mt-1 font-mono text-xs text-muted-foreground">{project.data.path}</p>
          </>
        ) : (
          <h1 className="text-2xl font-semibold tracking-tight">Project not found</h1>
        )}
      </header>

      {project.data && (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <Card className="p-4">
            <p className="text-xs uppercase tracking-wide text-muted-foreground">Memories</p>
            <p className="mt-1 text-xl font-semibold">{project.data.memory_count}</p>
          </Card>
          <Card className="p-4">
            <p className="text-xs uppercase tracking-wide text-muted-foreground">Last memory</p>
            <p className="mt-1 text-xs text-muted-foreground">
              {project.data.last_memory_at
                ? new Date(project.data.last_memory_at).toLocaleString()
                : "---"}
            </p>
          </Card>
          <Card className="p-4">
            <p className="text-xs uppercase tracking-wide text-muted-foreground">Last active</p>
            <p className="mt-1 text-xs text-muted-foreground">
              {new Date(project.data.last_active).toLocaleString()}
            </p>
          </Card>
          <Card className="p-4">
            <p className="text-xs uppercase tracking-wide text-muted-foreground">First seen</p>
            <p className="mt-1 text-xs text-muted-foreground">
              {new Date(project.data.first_seen).toLocaleString()}
            </p>
          </Card>
        </div>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Project memory</CardTitle>
          <CardDescription>
            Every Project-scoped memory registered under this project, most recent first.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {memories.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : memories.data && memories.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No project-scoped memories yet. An agent writes them via the{" "}
              <code className="font-mono">remember</code> tool with this project&apos;s{" "}
              <code className="font-mono">X-Cairn-Project</code> header.
            </p>
          ) : (
            <ul className="space-y-1.5">
              {memories.data?.map((m) => {
                const title = displayTitle(m.title, m.content);
                const showPreview = m.title && m.content.trim() !== title.trim();
                return (
                  <Item key={m.id} variant="outline" size="sm">
                    <ItemContent>
                      <ItemTitle className="line-clamp-1">{title}</ItemTitle>
                      {showPreview && (
                        <p className="line-clamp-1 text-xs text-muted-foreground">{m.content}</p>
                      )}
                      <ItemDescription className="flex items-center gap-2 flex-wrap">
                        <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                          {m.kind}
                        </Badge>
                        {m.promo_score > 0 && (
                          <Badge variant="secondary" className="mr-1.5 font-mono text-[10px]">
                            promo {m.promo_score.toFixed(2)}
                          </Badge>
                        )}
                        <span className="text-[11px] text-muted-foreground">
                          {m.tier} . {new Date(m.updated_at).toLocaleString()}
                        </span>
                      </ItemDescription>
                    </ItemContent>
                  </Item>
                );
              })}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Documents</CardTitle>
          <CardDescription>
            Reference material ingested from inside this project, plus anything global.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {documents.isLoading ? (
            <Skeleton className="h-10 w-full" />
          ) : documents.data && documents.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              Ingest from inside this repo:{" "}
              <code className="font-mono">cairn documents ingest &lt;path|url&gt;</code>.
            </p>
          ) : (
            <ul className="space-y-1.5">
              {documents.data?.map((d) => (
                <li
                  key={d.id}
                  className="flex items-center gap-2 text-xs rounded-md border border-line/40 bg-muted/30 px-3 py-2"
                >
                  <span className="font-medium">{d.title}</span>
                  <span className="font-mono text-muted-foreground truncate max-w-[200px]">
                    {d.source}
                  </span>
                  <Badge variant="outline" className="font-mono text-[10px]">
                    {d.chunk_count} chunks
                  </Badge>
                  {!d.project_id && (
                    <Badge variant="secondary" className="text-[10px]">
                      global
                    </Badge>
                  )}
                  <span className="ml-auto text-muted-foreground">
                    {new Date(d.updated_at).toLocaleString()}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Promotion activity</CardTitle>
          <CardDescription>
            Memories promoted out of (or demoted back into) this project&apos;s scope.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {promotionLog.isLoading ? (
            <Skeleton className="h-10 w-full" />
          ) : promotionActivity.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No promotion activity recorded for this project yet.
            </p>
          ) : (
            <ul className="space-y-1.5">
              {promotionActivity.map((entry) => (
                <li
                  key={entry.id}
                  className="flex items-center gap-2 text-xs rounded-md border border-line/40 bg-muted/30 px-3 py-2"
                >
                  <Badge
                    variant={entry.action === "promote" ? "secondary" : "outline"}
                    className="font-mono text-[10px] uppercase"
                  >
                    {entry.action}
                  </Badge>
                  <span className="font-mono truncate max-w-[140px]" title={entry.memory_id}>
                    {entry.memory_id}
                  </span>
                  <span className="text-muted-foreground">{entry.reason}</span>
                  <span className="text-muted-foreground">
                    {new Date(entry.ts).toLocaleString()}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
