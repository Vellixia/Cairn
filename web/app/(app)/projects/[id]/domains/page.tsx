"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { Building2, FolderGit2, Lightbulb, User } from "lucide-react";
import { api } from "@/lib/api";
import {
  ApiErrorState,
  NotRecorded,
  TruncationNotice,
  humanize,
} from "@/components/control-plane";
import { ListSkeleton, PageHeader, formatDate } from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

/** The page each panel asks for, so every panel is bounded (FR-895). */
const PAGE = 25;

/**
 * The three domains, plus patterns, kept visibly apart (FR-888).
 *
 * **Four panels rather than one merged list.** A project memory, a personal
 * note, a team standard and a personal pattern have different audiences and
 * different lifecycles, and a single list sorted by date would invite a reader
 * to treat one as another — which is precisely the confusion the domain
 * boundary exists to prevent. Each panel names its own audience in its own
 * subtitle.
 */
export default function DomainsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);

  const project = useQuery({
    queryKey: ["domain-project", id],
    queryFn: () => api.memories(id, { limit: PAGE }),
  });
  const personal = useQuery({
    queryKey: ["domain-personal"],
    queryFn: () => api.personalKnowledge({ limit: PAGE }),
  });
  const patterns = useQuery({
    queryKey: ["domain-patterns"],
    queryFn: () => api.patterns(),
  });
  const team = useQuery({
    queryKey: ["domain-team"],
    queryFn: () => api.teamKnowledge({ limit: PAGE }),
  });

  return (
    <div>
      <PageHeader
        title="Domains"
        subtitle="Where each kind of knowledge lives, and who it belongs to"
      />

      <div className="space-y-6">
        <Panel
          title="Project"
          icon={<FolderGit2 className="size-4" />}
          audience="Shared with everyone on this project."
          testId="domain-project"
          query={project}
        >
          {project.data && (
            <>
              <ul className="space-y-2" data-testid="domain-project-list">
                {project.data.memories.map((m) => (
                  <li key={m.id} data-testid="domain-project-row">
                    <Link
                      href={`/projects/${id}/memory/${m.id}`}
                      className="hover:bg-accent/50 block rounded-md px-2 py-1.5 transition"
                    >
                      <p className="text-sm">{m.content}</p>
                      <div className="text-muted-foreground mt-1 flex flex-wrap gap-2 text-xs">
                        <Badge variant="secondary">{m.type}</Badge>
                        <Badge variant="outline">{m.scope}</Badge>
                        <span>{formatDate(m.created_at)}</span>
                      </div>
                    </Link>
                  </li>
                ))}
              </ul>
              <Nothing when={project.data.memories.length === 0}>
                No project memory yet.
              </Nothing>
              <TruncationNotice
                shown={project.data.memories.length}
                limit={project.data.limit}
                refine="The memory explorer has the search and filters."
              />
            </>
          )}
        </Panel>

        {/*
          Personal knowledge and personal patterns are the signed-in account's
          own, and there is no control here that could ask for anybody else's.
          That is not a check this page performs — the two routes behind these
          panels take no owner parameter at all, not even for an administrator,
          so there is nothing for a selector to select (FR-708d).
        */}
        <Panel
          title="Personal"
          icon={<User className="size-4" />}
          audience="Yours alone. Nobody else can read this, including administrators."
          testId="domain-personal"
          query={personal}
        >
          {personal.data && (
            <>
              <ul className="space-y-2" data-testid="domain-personal-list">
                {personal.data.items.map((k) => (
                  <li
                    key={k.id}
                    className="rounded-md border p-2"
                    data-testid="domain-personal-row"
                  >
                    <p className="text-sm">{k.content}</p>
                    <div className="text-muted-foreground mt-1 flex flex-wrap items-center gap-2 text-xs">
                      <Badge variant="secondary">{k.knowledge_type}</Badge>
                      {k.topic_key && (
                        <code className="font-mono">
                          {k.topic_key}
                          {k.value_key ? ` = ${k.value_key}` : ""}
                        </code>
                      )}
                      <Applicability facts={k.applicability} />
                      <span>{formatDate(k.created_at)}</span>
                    </div>
                  </li>
                ))}
              </ul>
              <Nothing when={personal.data.items.length === 0}>
                You have no personal knowledge recorded.
              </Nothing>
              <TruncationNotice
                shown={personal.data.items.length}
                limit={personal.data.limit}
                refine="Older entries are reachable through the CLI."
              />
            </>
          )}
        </Panel>

        <Panel
          title="Patterns"
          icon={<Lightbulb className="size-4" />}
          audience="Personal-domain records of type pattern. Yours alone; a pattern never widens to a team by itself."
          testId="domain-patterns"
          query={patterns}
        >
          {patterns.data && (
            <>
              <ul className="space-y-2" data-testid="domain-patterns-list">
                {patterns.data.patterns.map((p) => (
                  <li
                    key={p.pattern_id}
                    className="rounded-md border p-2"
                    data-testid="domain-pattern-row"
                  >
                    <p className="text-sm font-medium">{p.title}</p>
                    <dl className="text-muted-foreground mt-1 space-y-0.5 text-xs">
                      <div>
                        <dt className="inline font-medium">Problem: </dt>
                        <dd className="inline">{p.problem}</dd>
                      </div>
                      <div>
                        <dt className="inline font-medium">Root cause: </dt>
                        <dd className="inline">{p.root_cause}</dd>
                      </div>
                      <div>
                        <dt className="inline font-medium">Approach: </dt>
                        <dd className="inline">{p.approach}</dd>
                      </div>
                    </dl>
                    <div className="text-muted-foreground mt-1 flex flex-wrap items-center gap-2 text-xs">
                      <Badge variant="outline">{humanize(p.trust)}</Badge>
                      <Applicability facts={p.applicability} />
                      <span>updated {formatDate(p.updated_at)}</span>
                    </div>
                  </li>
                ))}
              </ul>
              <Nothing when={patterns.data.patterns.length === 0}>
                You have promoted no patterns.
              </Nothing>
            </>
          )}
        </Panel>

        <Panel
          title="Team"
          icon={<Building2 className="size-4" />}
          audience="Server-wide guidance. Proposals are visible to their author and to administrators until ratified."
          testId="domain-team"
          query={team}
        >
          {team.data && (
            <>
              <ul className="space-y-2" data-testid="domain-team-list">
                {team.data.items.map((k) => (
                  <li
                    key={k.id}
                    className="rounded-md border p-2"
                    data-testid="domain-team-row"
                  >
                    <p className="text-sm">{k.content}</p>
                    <div className="text-muted-foreground mt-1 flex flex-wrap items-center gap-2 text-xs">
                      <Badge
                        variant={
                          k.state === "authoritative" ? "default" : "outline"
                        }
                        data-testid="domain-team-state"
                      >
                        {k.state}
                      </Badge>
                      <Badge variant="secondary">{k.knowledge_type}</Badge>
                      <Applicability facts={k.applicability} />
                      <span>{formatDate(k.created_at)}</span>
                    </div>
                  </li>
                ))}
              </ul>
              <Nothing when={team.data.items.length === 0}>
                No team knowledge is visible to you.
              </Nothing>
              <TruncationNotice
                shown={team.data.items.length}
                limit={team.data.limit}
                refine="Team curation lists the rest."
              />
            </>
          )}
        </Panel>
      </div>
    </div>
  );
}

function Panel({
  title,
  icon,
  audience,
  testId,
  query,
  children,
}: {
  title: string;
  icon: React.ReactNode;
  audience: string;
  testId: string;
  query: { isLoading: boolean; error: unknown };
  children: React.ReactNode;
}) {
  return (
    <Card data-testid={testId}>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-sm font-medium">
          {icon}
          {title}
        </CardTitle>
        <p className="text-muted-foreground text-xs">{audience}</p>
      </CardHeader>
      <CardContent>
        {query.isLoading && <ListSkeleton rows={2} />}
        {query.error != null && <ApiErrorState error={query.error} />}
        {children}
      </CardContent>
    </Card>
  );
}

/** An empty panel says so; it does not simply stop. */
function Nothing({
  when,
  children,
}: {
  when: boolean;
  children: React.ReactNode;
}) {
  if (!when) return null;
  return <p className="text-muted-foreground text-sm">{children}</p>;
}

/**
 * Which projects a record applies to.
 *
 * An empty list is not silence: a record with no applicability facts applies
 * everywhere, so "everywhere" is printed rather than nothing (D411, FR-435).
 */
function Applicability({
  facts,
}: {
  facts: { kind: string; value: string }[];
}) {
  if (!Array.isArray(facts) || facts.length === 0) {
    return <NotRecorded what="applies everywhere" />;
  }
  return (
    <span className="flex flex-wrap gap-1">
      {facts.map((f) => (
        <Badge key={`${f.kind}-${f.value}`} variant="outline">
          {f.kind}: {f.value}
        </Badge>
      ))}
    </span>
  );
}
