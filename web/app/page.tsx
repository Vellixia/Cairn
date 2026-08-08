"use client";

import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import { Card, Empty, ErrorNote, PageHeader } from "@/components/ui";

export default function ProjectsPage() {
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => api.projects(),
  });

  return (
    <main>
      <PageHeader
        title="Projects"
        subtitle="Projects you are a member of"
      />
      {projects.isLoading && <Empty>Loading…</Empty>}
      {projects.error != null && (
        <div>
          <ErrorNote error={projects.error} />
          <Link href="/login" className="text-sm underline">
            Sign in
          </Link>
        </div>
      )}
      {projects.data?.projects.length === 0 && (
        <Empty>
          No shared projects yet. Run <code>cairn link --create</code> in a
          repository to share one.
        </Empty>
      )}
      <ul className="space-y-2" data-testid="project-list">
        {projects.data?.projects.map((p) => (
          <li key={p.id}>
            <Link href={`/projects/${p.id}`}>
              <Card className="transition hover:border-neutral-400">
                <span className="font-medium">{p.name}</span>
                {p.repository_remote && (
                  <span className="ml-2 text-sm text-neutral-500">
                    {p.repository_remote}
                  </span>
                )}
              </Card>
            </Link>
          </li>
        ))}
      </ul>
    </main>
  );
}
