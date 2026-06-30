"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
  type SortingState,
} from "@tanstack/react-table";
import { Search, Upload, ShieldCheck, Globe, Users } from "lucide-react";
import Link from "next/link";
import { qk, usePublishPackMutation } from "@/lib/queries";
import { getJSON } from "@/lib/api";
import type { RegistryPackMeta } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogTrigger } from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";

function scopeBadge(scope: string) {
  const map: Record<string, { label: string; icon: typeof Globe }> = {
    public: { label: "Public", icon: Globe },
    team: { label: "Team", icon: Users },
    local: { label: "Local", icon: ShieldCheck },
  };
  const { label, icon: Icon } = map[scope] ?? { label: scope, icon: ShieldCheck };
  return (
    <Badge variant="outline" className="gap-1 font-mono text-[10px]">
      <Icon className="h-3 w-3" />
      {label}
    </Badge>
  );
}

const columns: ColumnDef<RegistryPackMeta>[] = [
  {
    accessorKey: "name",
    header: "Name",
    cell: ({ row }) => (
      <Link
        href={`/registry/packs/${encodeURIComponent(row.original.name)}`}
        className="font-medium underline-offset-2 hover:underline"
      >
        {row.original.name}
      </Link>
    ),
  },
  {
    accessorKey: "author",
    header: "Author",
    cell: ({ row }) => (
      <span className="text-sm text-muted-foreground">{row.original.author}</span>
    ),
  },
  {
    accessorKey: "version",
    header: "Version",
    cell: ({ row }) => (
      <code className="rounded bg-muted px-1.5 py-0.5 text-[11px] font-mono">
        {row.original.version}
      </code>
    ),
  },
  {
    accessorKey: "scope",
    header: "Scope",
    cell: ({ row }) => scopeBadge(row.original.scope ?? "public"),
  },
  {
    accessorKey: "has_ed25519_signature",
    header: "Signed",
    cell: ({ row }) =>
      row.original.has_ed25519_signature ? (
        <Badge className="text-[10px]">Signed</Badge>
      ) : (
        <Badge variant="secondary" className="text-[10px]">
          Unsigned
        </Badge>
      ),
  },
  {
    accessorKey: "download_count",
    header: "Downloads",
    cell: ({ row }) => (
      <span className="text-sm tabular-nums text-muted-foreground">
        {row.original.download_count}
      </span>
    ),
  },
  {
    accessorKey: "stored_at",
    header: "Published",
    cell: ({ row }) => (
      <span className="text-sm text-muted-foreground">
        {row.original.stored_at
          ? new Date(row.original.stored_at).toLocaleDateString()
          : "—"}
      </span>
    ),
  },
];

function PublishDialog() {
  const [open, setOpen] = useState(false);
  const [file, setFile] = useState<File | null>(null);
  const [trusted, setTrusted] = useState("");
  const publish = usePublishPackMutation();

  async function handlePublish() {
    if (!file) return;
    const buf = await file.arrayBuffer();
    publish.mutate(
      { tarball: buf, trusted: trusted || undefined },
      { onSuccess: () => setOpen(false) },
    );
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="default" size="sm">
          <Upload className="mr-1.5 h-4 w-4" />
          Publish
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Publish a pack</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div>
            <label className="mb-1 block text-sm font-medium">.cairnpkg file</label>
            <Input
              type="file"
              accept=".cairnpkg"
              onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            />
          </div>
          <div>
            <label className="mb-1 block text-sm font-medium">
              Trusted key (optional hex)
            </label>
            <Input
              placeholder="64-char hex public key"
              value={trusted}
              onChange={(e) => setTrusted(e.target.value)}
            />
          </div>
          <Button onClick={handlePublish} disabled={!file || publish.isPending} className="w-full">
            {publish.isPending ? "Publishing…" : "Publish"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export default function PacksContent() {
  const [search, setSearch] = useState("");
  const [sorting, setSorting] = useState<SortingState>([{ id: "stored_at", desc: true }]);

  const allPacks = useQuery({
    queryKey: qk.registryPacks,
    queryFn: () => getJSON<RegistryPackMeta[]>("/api/registry/packs"),
    refetchInterval: 30_000,
  });

  const searched = useQuery({
    queryKey: qk.registrySearch(search),
    queryFn: () =>
      getJSON<RegistryPackMeta[]>(`/registry/search?q=${encodeURIComponent(search)}`),
    enabled: search.length > 0,
  });

  const data = search.length > 0 ? (searched.data ?? []) : (allPacks.data ?? []);

  const table = useReactTable({
    data,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>Packs</CardTitle>
            <div className="flex items-center gap-2">
              <HelpButton content={HELP["/registry"]} />
              <PublishDialog />
            </div>
          </div>
          <div className="relative mt-2">
            <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
            <Input
              placeholder="Search packs…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              className="pl-8"
            />
          </div>
        </CardHeader>
        <CardContent>
          {allPacks.isLoading ? (
            <Skeleton className="h-64 w-full" />
          ) : data.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              {search.length > 0
                ? "No packs match your search."
                : "No packs published yet. Use the Publish button to upload a .cairnpkg file."}
            </p>
          ) : (
            <div className="overflow-x-auto rounded-md border border-line">
              <Table>
                <TableHeader>
                  {table.getHeaderGroups().map((hg) => (
                    <TableRow key={hg.id}>
                      {hg.headers.map((h) => (
                        <TableHead key={h.id}>
                          {flexRender(h.column.columnDef.header, h.getContext())}
                        </TableHead>
                      ))}
                    </TableRow>
                  ))}
                </TableHeader>
                <TableBody>
                  {table.getRowModel().rows.map((row) => (
                    <TableRow key={row.id}>
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
          )}
        </CardContent>
      </Card>
    </div>
  );
}
