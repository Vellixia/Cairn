"use client";

import { use } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, type HealthRow } from "@/lib/api";
import { ApiErrorState, humanize } from "@/components/control-plane";
import {
  EmptyState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

/**
 * How each status reads, and how loudly.
 *
 * **Six statuses, six meanings, and three of them are not failures.** A vendor
 * that does not offer a capability, a capability Cairn chose not to depend on,
 * and a capability nothing has been observed about are all quiet: none of them
 * is a fault to chase, and colouring them red would send somebody looking for a
 * bug in a working integration. `no_evidence` in particular is never rendered
 * as failing and never as working — it is the absence of an observation, which
 * is its own answer (FR-856).
 */
const STATUS_DISPLAY: Record<
  HealthRow["status"],
  { label: string; variant: "default" | "secondary" | "outline" | "destructive" }
> = {
  supported: { label: "working", variant: "default" },
  unsupported_by_vendor: {
    label: "not offered by this agent",
    variant: "outline",
  },
  declined_by_cairn: { label: "not enabled", variant: "outline" },
  adapter_unimplemented: { label: "not yet implemented", variant: "secondary" },
  runtime_failure: { label: "failing", variant: "destructive" },
  no_evidence: { label: "no evidence", variant: "outline" },
};

/** One day, for the window arithmetic below. */
const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * How long an observation about a capability stays a statement about now.
 *
 * **Per capability, because they are exercised at different rates.** Delivery
 * runs at every session open, so a week without one means the integration has
 * not been exercised recently and the last success says little about today. An
 * event kind fires only when the matching activity happens, and a fortnight of
 * ordinary work exercises the common ones. Receipt has no producer at all in
 * this feature, so its window is generous by design — there is nothing yet that
 * would refresh it.
 *
 * The server deliberately does not compute this: a single baked-in window would
 * assert that every capability goes stale at the same rate, which is the claim
 * this table exists to avoid making.
 */
function freshnessWindowDays(capability: string): number {
  if (capability.startsWith("deliver:")) return 7;
  if (capability === "receipt") return 30;
  return 14;
}

/**
 * Whether a row's observation has aged out of its window.
 *
 * A row with no `observed_at` is never stale: there is no observation to go out
 * of date, and marking it stale would turn "nothing was ever seen" into
 * "something was seen and has expired" (FR-860).
 */
function isStale(row: HealthRow): boolean {
  if (!row.observed_at) return false;
  const age = Date.now() - new Date(row.observed_at).getTime();
  return age > freshnessWindowDays(row.capability) * DAY_MS;
}

export default function AgentsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const health = useQuery({
    queryKey: ["integration-health", id],
    queryFn: () => api.integrationHealth(id),
  });

  const rows = health.data?.rows ?? [];

  // Grouped by agent *and* machine, because that is the identity of a cell. One
  // account on two laptops can legitimately see two different answers, and
  // collapsing them would let a working machine hide a broken one (FR-857).
  //
  // The group's parts are kept beside its rows rather than encoded into the key
  // and split back out: an agent name is free text and could contain whatever
  // separator a key format chose, and a group whose heading was split in the
  // wrong place would attribute a machine's rows to the wrong machine.
  //
  // Grouped by account as well as machine. `writer_id` is a label the reporting
  // client chooses, so two accounts can pick the same one — a shared CI name is
  // the obvious case — and grouping on the machine alone would put two people's
  // contradictory observations in one card with no way to tell them apart
  // (FR-857).
  const groups = new Map<
    string,
    { agent: string; writer: string; account: string; rows: HealthRow[] }
  >();
  for (const row of rows) {
    const key = JSON.stringify([row.agent, row.writer_id, row.account_id]);
    const existing = groups.get(key);
    if (existing) existing.rows.push(row);
    else
      groups.set(key, {
        agent: row.agent,
        writer: row.writer_id,
        account: row.account_id,
        rows: [row],
      });
  }

  return (
    <div>
      <PageHeader
        title="Agents"
        subtitle="What each agent, on each machine, has actually been observed doing"
      />

      {health.isLoading && <ListSkeleton rows={3} />}
      {health.error != null && <ApiErrorState error={health.error} />}

      {health.data && rows.length === 0 && (
        <EmptyState
          title="No health reported"
          description="No machine has filed an integration health report for this project yet. That is an absence of evidence, not a report of failure."
        />
      )}

      <div className="space-y-6" data-testid="agent-groups">
        {[...groups.entries()].map(([key, group]) => {
          return (
            <Card key={key} data-testid="agent-group" data-agent={group.agent} data-account={group.account}>
              <CardHeader>
                <CardTitle className="text-sm font-medium">
                  {group.agent}
                </CardTitle>
                <p className="text-muted-foreground text-xs">
                  machine{" "}
                  <code className="font-mono" data-testid="agent-writer">
                    {group.writer}
                  </code>{" "}
                  &middot; {group.rows.length} capabilit
                  {group.rows.length === 1 ? "y" : "ies"}
                </p>
              </CardHeader>
              <CardContent>
                <Table data-testid="health-table">
                  <TableHeader>
                    <TableRow>
                      <TableHead>Capability</TableHead>
                      <TableHead>Stage</TableHead>
                      <TableHead>Status</TableHead>
                      <TableHead>Evidence</TableHead>
                      <TableHead>Observed</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {group.rows.map((row) => (
                      <HealthTableRow
                        key={`${row.capability}-${row.stage}`}
                        row={row}
                      />
                    ))}
                  </TableBody>
                </Table>
              </CardContent>
            </Card>
          );
        })}
      </div>
    </div>
  );
}

function HealthTableRow({ row }: { row: HealthRow }) {
  const display = STATUS_DISPLAY[row.status] ?? {
    label: humanize(row.status),
    variant: "outline" as const,
  };
  const stale = isStale(row);

  return (
    <TableRow
      data-testid="health-row"
      data-capability={row.capability}
      data-status={row.status}
      data-stale={stale}
    >
      <TableCell className="font-mono text-xs">{row.capability}</TableCell>
      <TableCell className="text-muted-foreground text-xs">
        {humanize(row.stage)}
      </TableCell>
      <TableCell>
        <span className="flex flex-wrap items-center gap-1.5">
          <Badge variant={display.variant} data-testid="health-status">
            {display.label}
          </Badge>
          {row.degraded && <Badge variant="secondary">degraded</Badge>}
        </span>
      </TableCell>
      <TableCell>
        {/* Configuration read back and behaviour observed are different claims
            and get different badges: a file that says a hook is installed is
            not the hook having fired (FR-852, FR-853). */}
        {row.evidence_kind ? (
          <Badge variant="outline" data-testid="health-evidence-kind">
            {row.evidence_kind}
          </Badge>
        ) : (
          <span className="text-muted-foreground text-xs">none</span>
        )}
      </TableCell>
      <TableCell className="text-muted-foreground text-xs">
        {row.observed_at ? (
          <span data-testid="health-observed">
            {/* A `supported` row that has aged out reads "worked as of", never
                "working": an integration that functioned last month is not
                reported as functioning now on that basis alone (FR-860). */}
            {stale && row.status === "supported" ? "worked as of " : ""}
            {formatDate(row.observed_at)}
            {stale && (
              <Badge
                variant="secondary"
                className="ml-2"
                data-testid="health-stale"
              >
                stale
              </Badge>
            )}
          </span>
        ) : (
          <span data-testid="health-observed">never</span>
        )}
      </TableCell>
    </TableRow>
  );
}
