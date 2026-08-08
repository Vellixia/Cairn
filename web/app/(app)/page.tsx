"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { ChevronRight, FolderGit2 } from "lucide-react";
import { api } from "@/lib/api";
import {
  EmptyState,
  ErrorState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
import { Card } from "@/components/ui/card";

export default function ProjectsPage() {
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => api.projects(),
  });

  return (
    <div>
      <PageHeader title="Projects" subtitle="Projects you are a member of" />

      {projects.isLoading && <ListSkeleton />}
      {projects.error != null && <ErrorState error={projects.error} />}

      {projects.data?.projects.length === 0 && (
        <EmptyState
          title="No shared projects yet"
          description={
            <>
              Run{" "}
              <code className="bg-muted rounded px-1 py-0.5 font-mono text-xs">
                cairn link --create
              </code>{" "}
              inside a repository to share one.
            </>
          }
        />
      )}

      <ul className="grid gap-3 sm:grid-cols-2" data-testid="project-list">
        {projects.data?.projects.map((p) => (
          <li key={p.id}>
            <Link href={`/projects/${p.id}`} className="block">
              <Card className="hover:border-foreground/20 hover:bg-accent/40 group flex-row items-center gap-3 p-4 transition">
                <div className="bg-muted text-muted-foreground flex size-9 shrink-0 items-center justify-center rounded-md">
                  <FolderGit2 className="size-4" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="truncate font-medium">{p.name}</div>
                  <div className="text-muted-foreground truncate text-xs">
                    {p.repository_remote ?? `Created ${formatDate(p.created_at)}`}
                  </div>
                </div>
                <ChevronRight className="text-muted-foreground size-4 shrink-0 transition group-hover:translate-x-0.5" />
              </Card>
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}
