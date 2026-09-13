"use client";

import Link from "next/link";
import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, Pin, ShieldCheck } from "lucide-react";
import { api, type MemoryDetail } from "@/lib/api";
import {
  ApiErrorState,
  Field,
  NotRecorded,
  humanize,
} from "@/components/control-plane";
import { ListSkeleton, PageHeader, formatDate } from "@/components/page";
import { ReferenceChip } from "@/components/reference";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

export default function MemoryDetailPage({
  params,
}: {
  params: Promise<{ id: string; memoryId: string }>;
}) {
  const { id, memoryId } = use(params);
  const detail = useQuery({
    queryKey: ["memory", memoryId],
    queryFn: () => api.memory(memoryId),
  });

  const memory = detail.data?.memory;

  return (
    <div>
      <Button
        variant="ghost"
        size="sm"
        className="mb-3 -ml-2"
        render={<Link href={`/projects/${id}/memory`} />}
      >
        <ArrowLeft />
        All memory
      </Button>

      <PageHeader title="Memory" subtitle={memoryId} />

      {detail.isLoading && <ListSkeleton rows={4} />}
      {detail.error != null && <ApiErrorState error={detail.error} />}

      {memory && (
        <div className="space-y-4" data-testid="memory-detail">
          <Content memory={memory} />
          <Provenance memory={memory} projectId={id} />
          <Evidence memory={memory} />
          <Verification memory={memory} />
          <Relations memory={memory} projectId={id} />
          <Usage memory={memory} projectId={id} />
        </div>
      )}
    </div>
  );
}

/** What the knowledge says, and how it is classified (FR-884, FR-885). */
function Content({ memory }: { memory: MemoryDetail }) {
  return (
    <Card>
      <CardContent>
        <p className="text-sm" data-testid="detail-content">
          {memory.content}
        </p>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Badge variant="secondary">{memory.type}</Badge>
          <Badge variant="outline">
            {memory.scope}
            {memory.scope_key ? ` · ${memory.scope_key}` : ""}
          </Badge>
          <Badge variant="outline" data-testid="detail-state">
            {memory.state}
          </Badge>
          <Badge variant="outline" data-testid="detail-importance">
            {memory.importance}
          </Badge>
          {memory.pinned && (
            <Badge variant="outline">
              <Pin /> pinned
            </Badge>
          )}
        </div>
        <dl className="mt-4 grid gap-3 sm:grid-cols-3">
          <Field
            label="Origin"
            testId="detail-origin"
            value={
              // FR-885: whether somebody asked for this record or consolidation
              // produced it. Null is a record written before the column
              // existed; calling that "explicit" would assert an origin nobody
              // recorded.
              memory.origin_kind ? (
                humanize(memory.origin_kind)
              ) : (
                <NotRecorded what="not recorded for this record" />
              )
            }
          />
          <Field
            label="Reinforcements"
            testId="detail-reinforcements"
            value={memory.reinforcement_count}
          />
          <Field
            label="Superseded by"
            testId="detail-superseded-by"
            value={
              memory.superseded_by_id ? (
                <code className="font-mono text-xs">
                  {memory.superseded_by_id.slice(0, 8)}
                </code>
              ) : (
                <NotRecorded what="nothing" />
              )
            }
          />
          <Field label="Created" value={formatDate(memory.created_at)} />
          <Field label="Updated" value={formatDate(memory.updated_at)} />
        </dl>
      </CardContent>
    </Card>
  );
}

/** Where it came from. Identifiers, which is all the server holds. */
function Provenance({
  memory,
  projectId,
}: {
  memory: MemoryDetail;
  projectId: string;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Provenance</CardTitle>
      </CardHeader>
      <CardContent>
        <dl className="grid gap-3 sm:grid-cols-2">
          <Field
            label="Origin session"
            testId="detail-session"
            value={
              <Link
                href={`/projects/${projectId}/sessions/${memory.provenance.session_id}`}
                className="font-mono text-xs hover:underline"
              >
                {memory.provenance.session_id}
              </Link>
            }
          />
          <Field
            label="Observations"
            testId="detail-observations"
            value={`${memory.provenance.observation_ids.length} recorded`}
          />
        </dl>
      </CardContent>
    </Card>
  );
}

/**
 * What supports the record — and deliberately not the support itself.
 *
 * **Evidence content is never rendered here, and there is nothing to render.**
 * The server has never held the file contents, paths or command output behind
 * an observation; those stay on the machine that captured them (FR-055,
 * FR-061). So this section reports how much evidence exists and what kinds of
 * check looked at it, and then says where the material is — because an empty
 * section would read as "there is no evidence" when the truth is "the evidence
 * is not here" (FR-893).
 */
function Evidence({ memory }: { memory: MemoryDetail }) {
  const summary = memory.evidence_summary;
  const kinds = Array.isArray(summary.verifier_kinds)
    ? summary.verifier_kinds
    : [];
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Evidence</CardTitle>
      </CardHeader>
      <CardContent>
        <dl className="grid gap-3 sm:grid-cols-3">
          <Field
            label="Observations"
            testId="evidence-observations"
            value={summary.observation_count}
          />
          <Field
            label="Evidence items"
            testId="evidence-items"
            value={summary.evidence_count}
          />
          <Field
            label="Evidence facts"
            testId="evidence-facts"
            value={summary.evidence_fact_count}
          />
        </dl>
        <dl className="mt-3">
          <Field
            label="Verifier kinds"
            testId="evidence-verifier-kinds"
            value={
              kinds.length > 0 ? (
                <span className="flex flex-wrap gap-1.5">
                  {kinds.map((k) => (
                    <Badge key={String(k)} variant="outline">
                      {humanize(String(k))}
                    </Badge>
                  ))}
                </span>
              ) : (
                <NotRecorded what="no verifier has reported on this record" />
              )
            }
          />
        </dl>
        <p
          className="text-muted-foreground mt-4 text-xs"
          data-testid="evidence-local-notice"
        >
          Evidence content is local to session{" "}
          <code className="font-mono">
            {summary.local_to_session.slice(0, 8)}
          </code>{" "}
          and is not held on this server. Cairn keeps the counts and the kinds of
          check that ran, never the material they ran against.
        </p>
      </CardContent>
    </Card>
  );
}

/** Whether it is verified, and whether that is still true (FR-884). */
function Verification({ memory }: { memory: MemoryDetail }) {
  const v = memory.verification;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">Verification</CardTitle>
      </CardHeader>
      <CardContent>
        <dl className="grid gap-3 sm:grid-cols-3">
          <Field
            label="State"
            testId="verification-state"
            value={
              v.state ? (
                <Badge variant="outline">
                  <ShieldCheck /> {humanize(v.state)}
                </Badge>
              ) : (
                <NotRecorded what="never established" />
              )
            }
          />
          <Field
            label="Authority"
            testId="verification-authority"
            value={
              v.authority ? humanize(v.authority) : <NotRecorded what="none" />
            }
          />
          <Field
            label="Last verified"
            testId="verification-last"
            value={
              v.last_verified_at ? (
                formatDate(v.last_verified_at)
              ) : (
                <NotRecorded what="never" />
              )
            }
          />
        </dl>
        {v.stale && (
          <p
            className="text-muted-foreground mt-3 text-xs"
            data-testid="verification-stale"
          >
            This verification has expired. It was true when it was established;
            it is not a statement about the record now.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/** What it supersedes, what conflicts with it, what reinforces it (FR-884). */
function Relations({
  memory,
  projectId,
}: {
  memory: MemoryDetail;
  projectId: string;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm font-medium">
          Relations ({memory.relations.length})
        </CardTitle>
      </CardHeader>
      <CardContent>
        {memory.relations.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            Nothing supersedes, conflicts with or reinforces this record.
          </p>
        ) : (
          <ul className="space-y-2" data-testid="relation-list">
            {memory.relations.map((r) => (
              <li
                key={`${r.direction}-${r.kind}-${r.other.reference_key}`}
                className="flex flex-wrap items-center gap-2 text-sm"
                data-testid="relation-row"
              >
                <Badge variant="secondary" data-testid="relation-kind">
                  {humanize(r.kind)}
                </Badge>
                {/* Which end of the edge this record is on decides which
                    question the row answers, so it is stated rather than left
                    for a reader to work out from the ids. */}
                <Badge variant="outline" data-testid="relation-direction">
                  {r.direction}
                </Badge>
                <ReferenceChip reference={r.other} projectId={projectId} />
                <span className="text-muted-foreground text-xs">
                  {humanize(r.basis)} · {formatDate(r.decided_at)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

/** Where and when it has actually been retrieved (FR-884). */
function Usage({
  memory,
  projectId,
}: {
  memory: MemoryDetail;
  projectId: string;
}) {
  // The canonical key for a project memory. Written out rather than read from
  // the response because the detail route carries no self-reference — and the
  // domain is not a guess here: this route only ever serves project memories,
  // which is exactly why the two-part reference can be reconstructed safely.
  const referenceKey = `knowledge:project:${memory.id}`;
  return (
    <Card>
      <CardHeader className="flex flex-wrap items-center justify-between gap-2">
        <CardTitle className="text-sm font-medium">
          Retrieval usage ({memory.retrieval_usage.length})
        </CardTitle>
        <Button
          variant="link"
          size="xs"
          className="h-auto p-0"
          data-testid="usage-view-all"
          render={
            <Link
              href={`/projects/${projectId}/retrievals?reference_key=${encodeURIComponent(referenceKey)}`}
            />
          }
        >
          View all retrievals of this record
        </Button>
      </CardHeader>
      <CardContent>
        {memory.retrieval_usage.length === 0 ? (
          <p className="text-muted-foreground text-sm">
            No retrieval has considered this record yet.
          </p>
        ) : (
          <ul className="space-y-2" data-testid="usage-list">
            {memory.retrieval_usage.map((u) => (
              <li key={u.trace_id} data-testid="usage-row">
                <Link
                  href={`/projects/${projectId}/retrievals/${u.trace_id}`}
                  className="hover:bg-accent/50 flex flex-wrap items-center gap-2 rounded-md px-2 py-1.5 text-sm transition"
                >
                  <Badge
                    variant={u.status === "selected" ? "default" : "outline"}
                  >
                    {u.status}
                  </Badge>
                  <span className="text-muted-foreground text-xs">
                    {humanize(u.trigger)} at {humanize(u.delivery_point)}
                  </span>
                  <Badge variant="outline">{humanize(u.delivery_state)}</Badge>
                  <span className="text-muted-foreground ml-auto text-xs">
                    {formatDate(u.at)}
                  </span>
                </Link>
              </li>
            ))}
          </ul>
        )}
        {memory.retrieval_usage.length >= 20 && (
          <p className="text-muted-foreground mt-3 text-xs" data-testid="usage-truncated">
            The twenty most recent. The rest are on the retrievals list.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
