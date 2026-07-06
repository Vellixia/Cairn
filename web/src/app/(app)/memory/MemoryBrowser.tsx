"use client";

// Web redesign v2: the Memory Browser - EVERY memory Cairn has, filterable and sortable, with
// a full-detail drawer (all fields, provenance, edges). The agent writes and curates via MCP;
// this page is where humans watch. The only manual actions are housekeeping: pin + delete.

import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  useReactTable,
} from "@tanstack/react-table";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  useDeleteMemoryMutation,
  useMemoryListQuery,
  useMemoryQuery,
  usePinMemoryMutation,
  useWakeupQuery,
} from "@/lib/queries";
import type { Memory, MemoryListFilters } from "@/lib/api";
import { displayTitle } from "@/lib/memoryTitle";
import {
  Activity,
  AlertTriangle,
  ChevronLeft,
  ChevronRight,
  GitFork,
  LineChart,
  Network,
  Pin,
  PinOff,
  Search,
  Sparkles,
  Trash2,
} from "lucide-react";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 50;
const ALL = "__all__";

const INSIGHT_LINKS = [
  { href: "/memory/graph", label: "Graph", icon: Network },
  { href: "/memory/heatmap", label: "Heatmap", icon: Activity },
  { href: "/memory/savings", label: "Savings", icon: LineChart },
  { href: "/memory/architecture", label: "Architecture", icon: GitFork },
];

const TIER_OPTIONS = ["working", "episodic", "semantic", "procedural"];
const KIND_OPTIONS = ["fact", "decision", "task", "preference", "gotcha", "note"];
const SCOPE_OPTIONS = ["global", "project", "session"];
const SORT_OPTIONS: { value: NonNullable<MemoryListFilters["sort"]>; label: string }[] = [
  { value: "updated_at", label: "Recently updated" },
  { value: "created_at", label: "Recently created" },
  { value: "importance", label: "Importance" },
  { value: "promo_score", label: "Promotion score" },
  { value: "access_count", label: "Most accessed" },
];

function tierColor(tier: string): string {
  switch (tier) {
    case "semantic":
      return "border-blue-500/50 text-blue-600 dark:text-blue-300";
    case "procedural":
      return "border-purple-500/50 text-purple-600 dark:text-purple-300";
    case "episodic":
      return "border-amber-500/50 text-amber-600 dark:text-amber-300";
    default:
      return "border-line text-muted-foreground";
  }
}

export default function MemoryBrowser() {
  const router = useRouter();
  const params = useSearchParams();
  const searchRef = useRef<HTMLInputElement>(null);

  const [q, setQ] = useState("");
  const [debouncedQ, setDebouncedQ] = useState("");
  const [scope, setScope] = useState(ALL);
  const [tier, setTier] = useState(ALL);
  const [kind, setKind] = useState(ALL);
  const [pinnedOnly, setPinnedOnly] = useState(false);
  const [suspiciousOnly, setSuspiciousOnly] = useState(false);
  const [sort, setSort] = useState<NonNullable<MemoryListFilters["sort"]>>("updated_at");
  const [offset, setOffset] = useState(0);
  const [wakeupMode, setWakeupMode] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Legacy URL handling: old bookmarks / palette muscle memory. ?tab=graph etc. were the old
  // hub tabs; ?focus=search comes from the command palette's "Search memories" action.
  useEffect(() => {
    const tab = params.get("tab");
    if (tab) {
      if (["graph", "heatmap", "savings", "architecture"].includes(tab)) {
        router.replace(`/memory/${tab}`);
        return;
      }
      if (tab === "promotion") {
        router.replace("/automation");
        return;
      }
      router.replace("/memory"); // wakeup / recall / compression: the browser replaces them
      return;
    }
    if (params.get("focus") === "search") {
      searchRef.current?.focus();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params]);

  useEffect(() => {
    const t = setTimeout(() => setDebouncedQ(q.trim()), 300);
    return () => clearTimeout(t);
  }, [q]);

  // Consistent object shape (undefined for absent) so the query-key hash doesn't churn.
  const filters = useMemo<MemoryListFilters>(
    () => ({
      scope_type: scope === ALL ? undefined : scope,
      scope_id: undefined,
      tier: tier === ALL ? undefined : tier,
      kind: kind === ALL ? undefined : kind,
      pinned: pinnedOnly ? true : undefined,
      suspicious: suspiciousOnly ? true : undefined,
      q: debouncedQ.length > 0 ? debouncedQ : undefined,
      sort,
      limit: PAGE_SIZE,
      offset,
    }),
    [scope, tier, kind, pinnedOnly, suspiciousOnly, debouncedQ, sort, offset],
  );

  const list = useMemoryListQuery(filters);
  const wakeup = useWakeupQuery(50);

  const rows: Memory[] = wakeupMode ? (wakeup.data ?? []) : (list.data?.items ?? []);
  const total = wakeupMode ? (wakeup.data?.length ?? 0) : (list.data?.total ?? 0);
  const loading = wakeupMode ? wakeup.isLoading : list.isLoading;

  const columns = useMemo<ColumnDef<Memory>[]>(
    () => [
      {
        accessorKey: "content",
        header: "Memory",
        cell: ({ row }) => {
          const m = row.original;
          const title = displayTitle(m.title, m.content);
          // Only show a second, muted content line when the title is a REAL title (distinct
          // from content) - falling back to content-as-title would otherwise duplicate it.
          const showPreview = m.title && m.content.trim() !== title.trim();
          return (
            <div className="max-w-[420px]">
              <div className="flex items-center gap-1.5">
                {m.pinned && (
                  <Pin className="size-3 shrink-0 text-amber-500" aria-label="pinned" />
                )}
                {m.suspicious && (
                  <AlertTriangle
                    className="size-3 shrink-0 text-destructive"
                    aria-label="suspicious"
                  />
                )}
                <span className="truncate text-sm font-medium">{title}</span>
              </div>
              {showPreview && (
                <p className="truncate text-xs text-muted-foreground">{m.content}</p>
              )}
            </div>
          );
        },
      },
      {
        accessorKey: "kind",
        header: "Kind",
        cell: ({ row }) => (
          <Badge variant="outline" className="text-[10px]">
            {row.original.kind}
          </Badge>
        ),
      },
      {
        accessorKey: "tier",
        header: "Tier",
        cell: ({ row }) => (
          <Badge variant="outline" className={cn("text-[10px]", tierColor(row.original.tier))}>
            {row.original.tier}
          </Badge>
        ),
      },
      {
        accessorKey: "scope_type",
        header: "Scope",
        cell: ({ row }) => {
          const m = row.original;
          if (m.scope_type === "project" && m.scope_id) {
            return (
              <Link
                href={`/projects/${encodeURIComponent(m.scope_id)}`}
                onClick={(e) => e.stopPropagation()}
                className="text-xs underline decoration-dotted underline-offset-2 hover:text-foreground"
              >
                project
              </Link>
            );
          }
          return <span className="text-xs text-muted-foreground">{m.scope_type}</span>;
        },
      },
      {
        accessorKey: "importance",
        header: "Imp.",
        cell: ({ row }) => (
          <span className="font-mono text-xs tabular-nums">
            {row.original.importance.toFixed(2)}
          </span>
        ),
      },
      {
        accessorKey: "promo_score",
        header: "Promo",
        cell: ({ row }) => (
          <span className="font-mono text-xs tabular-nums text-muted-foreground">
            {row.original.promo_score.toFixed(2)}
          </span>
        ),
      },
      {
        accessorKey: "access_count",
        header: "Reads",
        cell: ({ row }) => (
          <span className="font-mono text-xs tabular-nums text-muted-foreground">
            {row.original.access_count}
          </span>
        ),
      },
      {
        accessorKey: "updated_at",
        header: "Updated",
        cell: ({ row }) => (
          <span className="text-xs text-muted-foreground whitespace-nowrap">
            {new Date(row.original.updated_at).toLocaleString()}
          </span>
        ),
      },
    ],
    [],
  );

  const table = useReactTable({
    data: rows,
    columns,
    getCoreRowModel: getCoreRowModel(),
    manualPagination: true,
  });

  const rangeStart = wakeupMode ? 1 : offset + 1;
  const rangeEnd = wakeupMode ? rows.length : offset + rows.length;

  return (
    <div className="space-y-6">
      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Memory</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Every memory Cairn has, with full provenance. The agent writes and curates; you
            watch.
          </p>
        </div>
        <div className="flex items-center gap-2">
          {INSIGHT_LINKS.map((l) => (
            <Button key={l.href} asChild variant="outline" size="sm">
              <Link href={l.href}>
                <l.icon className="size-3.5 mr-1" aria-hidden="true" />
                {l.label}
              </Link>
            </Button>
          ))}
          <HelpButton content={HELP["/memory"]} />
        </div>
      </header>

      <Card>
        <CardContent className="pt-4 space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative flex-1 min-w-[220px]">
              <Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
              <Input
                ref={searchRef}
                value={q}
                onChange={(e) => {
                  setQ(e.target.value);
                  setOffset(0);
                }}
                placeholder="Search content and concepts..."
                className="pl-8"
                disabled={wakeupMode}
              />
            </div>
            <Select
              value={scope}
              onValueChange={(v) => {
                setScope(v);
                setOffset(0);
              }}
              disabled={wakeupMode}
            >
              <SelectTrigger className="w-[130px]">
                <SelectValue placeholder="Scope" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL}>All scopes</SelectItem>
                {SCOPE_OPTIONS.map((s) => (
                  <SelectItem key={s} value={s}>
                    {s}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select
              value={tier}
              onValueChange={(v) => {
                setTier(v);
                setOffset(0);
              }}
              disabled={wakeupMode}
            >
              <SelectTrigger className="w-[130px]">
                <SelectValue placeholder="Tier" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL}>All tiers</SelectItem>
                {TIER_OPTIONS.map((t) => (
                  <SelectItem key={t} value={t}>
                    {t}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select
              value={kind}
              onValueChange={(v) => {
                setKind(v);
                setOffset(0);
              }}
              disabled={wakeupMode}
            >
              <SelectTrigger className="w-[130px]">
                <SelectValue placeholder="Kind" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL}>All kinds</SelectItem>
                {KIND_OPTIONS.map((k) => (
                  <SelectItem key={k} value={k}>
                    {k}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <Select
              value={sort}
              onValueChange={(v) => {
                setSort(v as NonNullable<MemoryListFilters["sort"]>);
                setOffset(0);
              }}
              disabled={wakeupMode}
            >
              <SelectTrigger className="w-[170px]">
                <SelectValue placeholder="Sort" />
              </SelectTrigger>
              <SelectContent>
                {SORT_OPTIONS.map((s) => (
                  <SelectItem key={s.value} value={s.value}>
                    {s.label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant={pinnedOnly ? "secondary" : "outline"}
              size="sm"
              disabled={wakeupMode}
              onClick={() => {
                setPinnedOnly((v) => !v);
                setOffset(0);
              }}
            >
              <Pin className="size-3.5 mr-1" aria-hidden="true" />
              Pinned
            </Button>
            <Button
              variant={suspiciousOnly ? "secondary" : "outline"}
              size="sm"
              disabled={wakeupMode}
              onClick={() => {
                setSuspiciousOnly((v) => !v);
                setOffset(0);
              }}
            >
              <AlertTriangle className="size-3.5 mr-1" aria-hidden="true" />
              Suspicious
            </Button>
            <div className="flex-1" />
            <Button
              variant={wakeupMode ? "secondary" : "outline"}
              size="sm"
              onClick={() => setWakeupMode((v) => !v)}
              title="Preview the ranked list a fresh agent session would load first"
            >
              <Sparkles className="size-3.5 mr-1" aria-hidden="true" />
              Wakeup order
            </Button>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardContent className="pt-4">
          {loading ? (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : rows.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              {debouncedQ || scope !== ALL || tier !== ALL || kind !== ALL
                ? "No memories match these filters."
                : "No memories yet. They appear here as agents remember things."}
            </p>
          ) : (
            <>
              <div className="rounded-md border border-line overflow-x-auto">
                <Table>
                  <TableHeader>
                    {table.getHeaderGroups().map((hg) => (
                      <TableRow key={hg.id}>
                        {hg.headers.map((h) => (
                          <TableHead key={h.id}>
                            {h.isPlaceholder
                              ? null
                              : flexRender(h.column.columnDef.header, h.getContext())}
                          </TableHead>
                        ))}
                      </TableRow>
                    ))}
                  </TableHeader>
                  <TableBody>
                    {table.getRowModel().rows.map((row) => (
                      <TableRow
                        key={row.id}
                        className="cursor-pointer"
                        onClick={() => setSelectedId(row.original.id)}
                      >
                        {row.getVisibleCells().map((cell) => (
                          <TableCell key={cell.id}>
                            {flexRender(cell.column.columnDef.cell, cell.getContext())}
                          </TableCell>
                        ))}
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
              <div className="mt-3 flex items-center justify-between text-xs text-muted-foreground">
                <span>
                  {rangeStart}-{rangeEnd} of {total}
                  {wakeupMode && " (wakeup ranking)"}
                </span>
                {!wakeupMode && (
                  <div className="flex items-center gap-1">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={offset === 0}
                      onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
                    >
                      <ChevronLeft className="size-3.5" aria-hidden="true" />
                      Prev
                    </Button>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={offset + PAGE_SIZE >= total}
                      onClick={() => setOffset(offset + PAGE_SIZE)}
                    >
                      Next
                      <ChevronRight className="size-3.5" aria-hidden="true" />
                    </Button>
                  </div>
                )}
              </div>
            </>
          )}
        </CardContent>
      </Card>

      <MemoryDetailDrawer selectedId={selectedId} onSelect={setSelectedId} />
    </div>
  );
}

// ---- detail drawer ------------------------------------------------------------

function MemoryDetailDrawer({
  selectedId,
  onSelect,
}: {
  selectedId: string | null;
  onSelect: (id: string | null) => void;
}) {
  const detail = useMemoryQuery(selectedId);
  const pin = usePinMemoryMutation();
  const del = useDeleteMemoryMutation();
  const [confirmDelete, setConfirmDelete] = useState(false);
  const m = detail.data;

  return (
    <>
      <Sheet open={selectedId !== null} onOpenChange={(o) => !o && onSelect(null)}>
        <SheetContent side="right" className="w-full sm:max-w-xl overflow-y-auto">
          {detail.isLoading || !m ? (
            <div className="space-y-3 pt-8">
              <Skeleton className="h-6 w-2/3" />
              <Skeleton className="h-24 w-full" />
              <Skeleton className="h-6 w-1/2" />
            </div>
          ) : (
            <>
              <SheetHeader>
                <SheetTitle className="flex items-center gap-2 text-left">
                  {m.pinned && <Pin className="size-4 text-amber-500" aria-label="pinned" />}
                  {displayTitle(m.title, m.content)}
                </SheetTitle>
                <SheetDescription className="font-mono text-[11px] break-all text-left">
                  {m.id}
                </SheetDescription>
              </SheetHeader>

              <div className="mt-4 space-y-5 pb-8">
                {m.suspicious && (
                  <div className="flex items-start gap-2 rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                    <AlertTriangle className="size-4 shrink-0" aria-hidden="true" />
                    Flagged suspicious by the content scanner (possible secret or injected
                    instruction). Agents see this flag on recall.
                  </div>
                )}

                <section>
                  <p className="text-xs uppercase tracking-wide text-muted-foreground mb-1.5">
                    Content
                  </p>
                  <p className="whitespace-pre-wrap rounded-md border border-line bg-muted/30 p-3 text-sm">
                    {m.content}
                  </p>
                </section>

                {m.reasoning && (
                  <section>
                    <p className="text-xs uppercase tracking-wide text-muted-foreground mb-1.5">
                      Reasoning
                    </p>
                    <p className="whitespace-pre-wrap rounded-md border border-line bg-muted/20 p-3 text-sm text-muted-foreground">
                      {m.reasoning}
                    </p>
                  </section>
                )}

                <section className="flex flex-wrap gap-1.5">
                  <Badge variant="outline" className="text-[10px]">
                    {m.kind}
                  </Badge>
                  <Badge variant="outline" className={cn("text-[10px]", tierColor(m.tier))}>
                    {m.tier}
                  </Badge>
                  {m.scope_type === "project" && m.scope_id ? (
                    <Badge variant="outline" className="text-[10px]">
                      <Link
                        href={`/projects/${encodeURIComponent(m.scope_id)}`}
                        className="underline decoration-dotted underline-offset-2"
                      >
                        project: {m.scope_id.slice(0, 12)}
                      </Link>
                    </Badge>
                  ) : (
                    <Badge variant="outline" className="text-[10px]">
                      {m.scope_type}
                      {m.scope_id ? `: ${m.scope_id.slice(0, 12)}` : ""}
                    </Badge>
                  )}
                  {m.promo_locked && (
                    <Badge variant="outline" className="text-[10px]">
                      promo locked
                    </Badge>
                  )}
                </section>

                <section className="grid grid-cols-2 gap-x-6 gap-y-2 text-xs sm:grid-cols-3">
                  <Metric label="Importance" value={m.importance.toFixed(2)} />
                  <Metric label="Confidence" value={m.confidence.toFixed(2)} />
                  <Metric label="Promo score" value={m.promo_score.toFixed(2)} />
                  <Metric label="Reads" value={`${m.access_count}`} />
                  <Metric label="Created" value={new Date(m.created_at).toLocaleString()} />
                  <Metric label="Updated" value={new Date(m.updated_at).toLocaleString()} />
                </section>

                {m.session_id && (
                  <section className="text-xs">
                    <p className="uppercase tracking-wide text-muted-foreground mb-1">
                      Provenance
                    </p>
                    <p className="font-mono text-[11px] text-muted-foreground break-all">
                      session {m.session_id}
                    </p>
                  </section>
                )}

                {m.concepts.length > 0 && (
                  <section>
                    <p className="text-xs uppercase tracking-wide text-muted-foreground mb-1.5">
                      Concepts
                    </p>
                    <div className="flex flex-wrap gap-1">
                      {m.concepts.map((c) => (
                        <Badge key={c} variant="secondary" className="text-[10px]">
                          {c}
                        </Badge>
                      ))}
                    </div>
                  </section>
                )}

                {m.files.length > 0 && (
                  <section>
                    <p className="text-xs uppercase tracking-wide text-muted-foreground mb-1.5">
                      Files
                    </p>
                    <ul className="space-y-0.5">
                      {m.files.map((f) => (
                        <li key={f} className="font-mono text-[11px] text-muted-foreground">
                          {f}
                        </li>
                      ))}
                    </ul>
                  </section>
                )}

                <EdgeSection label="Derived from" ids={m.derived_from} onSelect={onSelect} />
                <EdgeSection label="Contradicts" ids={m.contradicts} onSelect={onSelect} />
                <EdgeSection label="Supersedes" ids={m.supersedes} onSelect={onSelect} />
                <AppliesToSection values={m.applies_to} />

                <section className="flex items-center gap-2 border-t border-line pt-4">
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={pin.isPending}
                    onClick={() => pin.mutate({ id: m.id, pinned: !m.pinned })}
                  >
                    {m.pinned ? (
                      <>
                        <PinOff className="size-3.5 mr-1" aria-hidden="true" /> Unpin
                      </>
                    ) : (
                      <>
                        <Pin className="size-3.5 mr-1" aria-hidden="true" /> Pin
                      </>
                    )}
                  </Button>
                  <div className="flex-1" />
                  <Button
                    variant="ghost"
                    size="sm"
                    className="text-destructive hover:text-destructive"
                    onClick={() => setConfirmDelete(true)}
                  >
                    <Trash2 className="size-3.5 mr-1" aria-hidden="true" />
                    Delete
                  </Button>
                </section>
              </div>
            </>
          )}
        </SheetContent>
      </Sheet>

      <AlertDialog open={confirmDelete} onOpenChange={setConfirmDelete}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this memory?</AlertDialogTitle>
            <AlertDialogDescription>
              Permanent - memories are not covered by the lossless-retention guarantee (that
              covers compressed file reads). The agent can re-learn it, but this record and its
              edges are gone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={del.isPending}
              onClick={() => {
                setConfirmDelete(false);
                if (m) {
                  del.mutate(m.id, { onSuccess: () => onSelect(null) });
                }
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className="uppercase tracking-wide text-muted-foreground">{label}</p>
      <p className="mt-0.5 font-mono tabular-nums">{value}</p>
    </div>
  );
}

// Edges hold memory ids - clicking hops the drawer to that memory in place.
function EdgeSection({
  label,
  ids,
  onSelect,
}: {
  label: string;
  ids: string[];
  onSelect: (id: string) => void;
}) {
  if (ids.length === 0) return null;
  return (
    <section>
      <p className="text-xs uppercase tracking-wide text-muted-foreground mb-1.5">{label}</p>
      <div className="flex flex-wrap gap-1">
        {ids.map((id) => (
          <button
            key={id}
            type="button"
            onClick={() => onSelect(id)}
            className="rounded border border-line bg-muted/40 px-1.5 py-0.5 font-mono text-[10px] hover:bg-muted"
          >
            {id.slice(0, 12)}…
          </button>
        ))}
      </div>
    </section>
  );
}

// applies_to entries are file paths / symbols / project ids - plain text, not hoppable.
function AppliesToSection({ values }: { values: string[] }) {
  if (values.length === 0) return null;
  return (
    <section>
      <p className="text-xs uppercase tracking-wide text-muted-foreground mb-1.5">Applies to</p>
      <ul className="space-y-0.5">
        {values.map((v) => (
          <li key={v} className="font-mono text-[11px] text-muted-foreground break-all">
            {v}
          </li>
        ))}
      </ul>
    </section>
  );
}
