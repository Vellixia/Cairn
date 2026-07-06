"use client";

import { useQuery } from "@tanstack/react-query";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { getJSON } from "@/lib/api";
import { useUrlId } from "@/lib/useUrlId";

interface Session {
  id: string;
  project_hash: string;
  started_at: string;
  ended_at: string | null;
  tasks: Array<{ id: string; title: string; progress: string }>;
  findings: Array<{ text: string; source_file?: string; confidence: number }>;
  decisions: Array<{ text: string; rationale: string; confidence: number }>;
  touched_files: Array<{ path: string; mode: string }>;
  next_steps: string[];
  memory_ids: string[];
}

export default function SessionDetail() {
  const id = useUrlId() ?? "new";
  const session = useQuery({
    queryKey: ["sessions", id],
    queryFn: () => getJSON<Session>(`/api/sessions/${id}`),
    enabled: id !== "new",
  });

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Session</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          <code className="font-mono">{id}</code>
        </p>
      </header>
      {id === "new" ? (
        <Card className="p-6 text-sm text-muted-foreground">
          Select a session from <a href="/dashboard/sessions" className="underline">the listing</a> to see details.
        </Card>
      ) : session.isLoading ? (
        <Skeleton className="h-72 w-full" />
      ) : session.data ? (
        <SessionView s={session.data} />
      ) : (
        <p className="text-sm text-muted-foreground">Session not found.</p>
      )}
    </div>
  );
}

function SessionView({ s }: { s: Session }) {
  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>CCP block</CardTitle>
          <CardDescription>
            Compact form of this session --- what `SessionStart` injects.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <pre className="max-h-96 overflow-auto rounded-md border border-line bg-secondary p-3 font-mono text-xs leading-relaxed">
            {ccpBlock(s)}
          </pre>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Tasks</CardTitle>
          <CardDescription>{s.tasks.length} total</CardDescription>
        </CardHeader>
        <CardContent>
          {s.tasks.length === 0 ? (
            <p className="text-sm text-muted-foreground">None recorded.</p>
          ) : (
            <ul className="space-y-1">
              {s.tasks.map((t) => (
                <li key={t.id} className="text-sm">
                  <span className="font-mono text-[10px] text-muted-foreground mr-2">
                    {t.id}
                  </span>
                  {t.title}{" "}
                  <span className="text-xs text-muted-foreground">--- {t.progress}</span>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Findings</CardTitle>
        </CardHeader>
        <CardContent>
          {s.findings.length === 0 ? (
            <p className="text-sm text-muted-foreground">None recorded.</p>
          ) : (
            <ul className="space-y-2">
              {s.findings.map((f, i) => (
                <li key={i} className="text-sm">
                  {f.text}
                  {f.source_file && (
                    <span className="ml-1 text-[10px] font-mono text-muted-foreground">
                      ({f.source_file})
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Decisions</CardTitle>
        </CardHeader>
        <CardContent>
          {s.decisions.length === 0 ? (
            <p className="text-sm text-muted-foreground">None recorded.</p>
          ) : (
            <ul className="space-y-2">
              {s.decisions.map((d, i) => (
                <li key={i} className="text-sm">
                  <Badge variant="secondary" className="mr-2 font-mono text-[10px]">
                    decision
                  </Badge>
                  {d.text}
                  <p className="ml-6 text-xs text-muted-foreground">
                    rationale: {d.rationale}
                  </p>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Next steps</CardTitle>
        </CardHeader>
        <CardContent>
          {s.next_steps.length === 0 ? (
            <p className="text-sm text-muted-foreground">None recorded.</p>
          ) : (
            <ul className="list-disc pl-5 space-y-1 text-sm">
              {s.next_steps.map((n, i) => (
                <li key={i}>{n}</li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </>
  );
}

function ccpBlock(s: Session) {
  let out = `# Cross-Session Protocol --- session ${s.id}\n`;
  out += `Project: ${s.project_hash}\n`;
  out += `Started: ${s.started_at}\n`;
  if (s.ended_at) out += `Ended: ${s.ended_at}\n`;
  if (s.tasks.length) {
    out += `\n## Tasks\n`;
    for (const t of s.tasks) out += `- [${t.id}] ${t.title} --- ${t.progress}\n`;
  }
  if (s.findings.length) {
    out += `\n## Findings\n`;
    for (const f of s.findings) {
      out += `- ${f.text}${f.source_file ? ` (from ${f.source_file})` : ""}\n`;
    }
  }
  if (s.decisions.length) {
    out += `\n## Decisions\n`;
    for (const d of s.decisions) out += `- ${d.text} (rationale: ${d.rationale})\n`;
  }
  if (s.touched_files.length) {
    out += `\n## Touched files\n`;
    for (const f of s.touched_files) out += `- ${f.path} (${f.mode})\n`;
  }
  if (s.next_steps.length) {
    out += `\n## Next steps\n`;
    for (const n of s.next_steps) out += `- ${n}\n`;
  }
  return out;
}
