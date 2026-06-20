"use client";

import Link from "next/link";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { Controller } from "react-hook-form";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Field,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Item, ItemContent, ItemTitle, ItemDescription } from "@/components/ui/item";
import {
  useAnchorQuery,
  useSetAnchorMutation,
  useStatsQuery,
  useWakeupQuery,
  useDevicesAuditQuery,
} from "@/lib/queries";
import { anchorSchema, type AnchorInput } from "@/lib/forms/schemas";

export default function DashboardOverviewPage() {
  const stats = useStatsQuery();
  const memories = useWakeupQuery(5);
  const audit = useDevicesAuditQuery();
  const anchor = useAnchorQuery();
  const rel = stats.data?.reliability;

  const scoreColor =
    !rel
      ? "text-muted-foreground"
      : rel.score >= 80
      ? "text-emerald-500"
      : rel.score >= 50
      ? "text-amber-500"
      : "text-destructive";

  return (
    <div className="space-y-6">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Overview</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Server health, reliability, recent memory, and the last few admin actions.
        </p>
      </header>

      <section className="grid gap-4 md:grid-cols-3">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground font-normal">
              Server
            </CardTitle>
            {stats.isLoading && <Skeleton className="h-3 w-8" />}
          </CardHeader>
          <CardContent className="space-y-1.5 text-sm">
            <Stat k="Status" v={stats.data ? "ok" : "…"} />
            <Stat k="Memories" v={stats.data ? String(stats.data.memories) : "…"} />
            <Stat
              k="Checkpoints"
              v={
                stats.data?.checkpoints != null
                  ? String(stats.data.checkpoints)
                  : "…"
              }
            />
            <Stat
              k="Preferences"
              v={
                stats.data?.preferences != null
                  ? String(stats.data.preferences)
                  : "…"
              }
            />
            <Stat
              k="Anchor"
              v={stats.data?.anchor ? `"${stats.data.anchor}"` : "none"}
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
            <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground font-normal">
              Reliability
            </CardTitle>
            {stats.isLoading && <Skeleton className="h-3 w-8" />}
          </CardHeader>
          <CardContent>
            {rel ? (
              <>
                <div className={`text-4xl font-bold ${scoreColor}`}>
                  {rel.score}
                  <span className="text-base text-muted-foreground">/100</span>
                </div>
                <p className="mt-1 text-xs text-muted-foreground">
                  {rel.samples} edit{rel.samples === 1 ? "" : "s"} ·{" "}
                  <span className="text-emerald-500">{rel.ok} ok</span> ·{" "}
                  <span className="text-amber-500">{rel.warn} warn</span> ·{" "}
                  <span className="text-destructive">{rel.danger} danger</span> ·{" "}
                  {rel.rollbacks} rollback
                  {rel.rollbacks === 1 ? "" : "s"}
                </p>
              </>
            ) : (
              <p className="text-sm text-muted-foreground">No edit history yet.</p>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground font-normal">
              Quick actions
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-2 gap-2">
              <Button asChild variant="outline" size="sm">
                <Link href="/dashboard/memory">Remember</Link>
              </Button>
              <Button asChild variant="outline" size="sm">
                <Link href="/dashboard/memory/recall">Recall</Link>
              </Button>
              <Button asChild variant="outline" size="sm">
                <Link href="/dashboard/share/sanitize">Sanitize</Link>
              </Button>
              <Button asChild variant="outline" size="sm">
                <Link href="/dashboard/devices">Issue token</Link>
              </Button>
            </div>
            <p className="mt-3 text-[11px] text-muted-foreground">
              ⌘K opens the command palette. <code>?</code> shows shortcuts.
            </p>
          </CardContent>
        </Card>
      </section>

      <section className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground font-normal">
              Recent memory
            </CardTitle>
          </CardHeader>
          <CardContent>
            {memories.isLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-12 w-full" />
                <Skeleton className="h-12 w-full" />
                <Skeleton className="h-12 w-full" />
              </div>
            ) : memories.data && memories.data.length === 0 ? (
              <p className="text-sm text-muted-foreground">No memories yet.</p>
            ) : memories.data ? (
              <ul className="space-y-1.5">
                {memories.data.slice(0, 5).map((m) => (
                  <Item key={m.id} variant="outline" size="sm">
                    <ItemContent>
                      <ItemTitle className="line-clamp-2">{m.content}</ItemTitle>
                      <ItemDescription>
                        <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                          {m.kind}
                        </Badge>
                        {m.tier} · {new Date(m.created_at).toLocaleString()}
                      </ItemDescription>
                    </ItemContent>
                  </Item>
                ))}
              </ul>
            ) : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground font-normal">
              Recent admin events
            </CardTitle>
          </CardHeader>
          <CardContent>
            {audit.isLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-8 w-full" />
                <Skeleton className="h-8 w-full" />
                <Skeleton className="h-8 w-full" />
              </div>
            ) : audit.data && audit.data.length === 0 ? (
              <p className="text-sm text-muted-foreground">No events recorded yet.</p>
            ) : audit.data ? (
              <ul className="space-y-1.5 text-sm">
                {audit.data.slice(0, 8).map((e, i) => {
                  const isError =
                    e.kind.startsWith("login_failed") || e.kind === "token_revoked";
                  const isOk = e.kind.startsWith("login_ok") || e.kind === "setup";
                  return (
                    <li
                      key={i}
                      className="flex items-baseline gap-3 rounded-md border border-line px-3 py-1.5"
                    >
                      <Badge
                        variant={isError ? "destructive" : isOk ? "secondary" : "outline"}
                        className="font-mono text-[10px] uppercase tracking-wider"
                      >
                        {e.kind}
                      </Badge>
                      <span className="flex-1 text-muted-foreground truncate">
                        {e.detail}
                      </span>
                      <span className="text-[11px] text-muted-foreground">
                        {relativeTime(e.ts)}
                      </span>
                    </li>
                  );
                })}
              </ul>
            ) : null}
            <p className="mt-2 text-[11px] text-muted-foreground">
              In-memory ring buffer; lost on restart.
            </p>
          </CardContent>
        </Card>
      </section>

      <Card>
        <CardHeader>
          <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground font-normal">
            Set a task anchor
          </CardTitle>
        </CardHeader>
        <CardContent>
          <AnchorEditor
            current={anchor.data?.anchor ?? null}
            onSaved={() => {
              anchor.refetch();
              stats.refetch();
            }}
          />
        </CardContent>
      </Card>
    </div>
  );
}

function Stat({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between border-b border-dashed border-line py-1.5 text-sm last:border-0">
      <span className="text-muted-foreground">{k}</span>
      <span className="font-mono text-teal truncate max-w-[60%]">{v}</span>
    </div>
  );
}

function AnchorEditor({
  current,
  onSaved,
}: {
  current: string | null;
  onSaved: () => void;
}) {
  const setAnchor = useSetAnchorMutation();
  const form = useForm<AnchorInput>({
    resolver: zodResolver(anchorSchema),
    defaultValues: { goal: current ?? "" },
  });
  async function onSubmit(values: AnchorInput) {
    try {
      await setAnchor.mutateAsync(values);
      form.reset({ goal: values.goal });
      onSaved();
    } catch {
      /* toast handled in mutation */
    }
  }
  return (
    <div className="space-y-2">
      {current && (
        <p className="rounded-md border border-line bg-secondary px-3 py-2 text-sm">
          {current}
        </p>
      )}
      <form
        id="form-anchor"
        onSubmit={form.handleSubmit(onSubmit)}
        className="flex gap-2"
      >
        <FieldGroup className="flex-1 flex-row gap-2">
          <Controller
            name="goal"
            control={form.control}
            render={({ field, fieldState }) => (
              <Field data-invalid={fieldState.invalid} className="flex-1">
                <FieldLabel htmlFor="form-anchor-goal" className="sr-only">
                  Goal
                </FieldLabel>
                <Input
                  {...field}
                  id="form-anchor-goal"
                  aria-invalid={fieldState.invalid}
                  placeholder='e.g. "Ship the HelixDB backend behind the store seam"'
                />
                {fieldState.invalid && (
                  <FieldError errors={[fieldState.error]} />
                )}
              </Field>
            )}
          />
          <Button type="submit" form="form-anchor" disabled={setAnchor.isPending}>
            {current ? "Update" : "Set"}
          </Button>
        </FieldGroup>
      </form>
    </div>
  );
}

function relativeTime(ts: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}
