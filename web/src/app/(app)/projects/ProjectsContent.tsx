"use client";

import Link from "next/link";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import {
  type ColumnDef,
  flexRender,
  getCoreRowModel,
  useReactTable,
} from "@tanstack/react-table";
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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useProjectsQuery } from "@/lib/queries";
import type { ProjectWithStats } from "@/lib/api";

function relativeOrDash(ts: string | null): string {
  if (!ts) return "---";
  return new Date(ts).toLocaleString();
}

export default function ProjectsContent() {
  const projects = useProjectsQuery();

  const columns: ColumnDef<ProjectWithStats>[] = [
    {
      accessorKey: "name",
      header: "Project",
      cell: ({ row }) => (
        <Link
          href={`/projects/${encodeURIComponent(row.original.id)}`}
          className="font-medium underline underline-offset-2"
        >
          {row.original.name}
        </Link>
      ),
    },
    {
      accessorKey: "path",
      header: "Path",
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground">{row.original.path}</span>
      ),
    },
    {
      accessorKey: "memory_count",
      header: "Memories",
      cell: ({ row }) => (
        <Badge variant="outline" className="font-mono text-[10px]">
          {row.original.memory_count}
        </Badge>
      ),
    },
    {
      accessorKey: "last_memory_at",
      header: "Last memory",
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground">
          {relativeOrDash(row.original.last_memory_at)}
        </span>
      ),
    },
    {
      accessorKey: "last_active",
      header: "Last active",
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground">
          {new Date(row.original.last_active).toLocaleString()}
        </span>
      ),
    },
  ];

  const table = useReactTable({
    data: projects.data ?? [],
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  return (
    <div className="space-y-6 max-w-4xl">
      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Projects</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Repos an agent with Cairn hooks has worked in, auto-detected and registered on
            session start.
          </p>
        </div>
        <HelpButton content={HELP["/projects"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Registered projects</CardTitle>
          <CardDescription>
            {projects.data ? `${projects.data.length} project(s)` : "Loading..."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {projects.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : projects.data && projects.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              Projects are auto-detected when an agent with Cairn hooks starts a session in a
              repo.
            </p>
          ) : (
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
