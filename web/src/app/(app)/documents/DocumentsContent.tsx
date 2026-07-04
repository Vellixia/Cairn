"use client";

import { useState } from "react";
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
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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
import { Trash2, Search } from "lucide-react";
import Link from "next/link";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  useDocumentsQuery,
  useDocumentSearchQuery,
  useDeleteDocumentMutation,
  useProjectsQuery,
} from "@/lib/queries";
import type { DocumentSummary } from "@/lib/api";

const ALL_PROJECTS = "__all__";

export default function DocumentsContent() {
  const projects = useProjectsQuery();
  const [projectFilter, setProjectFilter] = useState(ALL_PROJECTS);
  const activeProject = projectFilter === ALL_PROJECTS ? undefined : projectFilter;
  const documents = useDocumentsQuery(activeProject);
  const del = useDeleteDocumentMutation();
  const [query, setQuery] = useState("");
  const search = useDocumentSearchQuery(query, 10, activeProject);
  const [deleteTarget, setDeleteTarget] = useState<DocumentSummary | null>(null);

  const projectName = (id: string | null) =>
    id ? (projects.data?.find((p) => p.id === id)?.name ?? id) : null;

  const columns: ColumnDef<DocumentSummary>[] = [
    {
      accessorKey: "title",
      header: "Title",
      cell: ({ row }) => <span className="font-medium">{row.original.title}</span>,
    },
    {
      accessorKey: "source",
      header: "Source",
      cell: ({ row }) => (
        <span className="font-mono text-xs text-muted-foreground truncate block max-w-[200px]">
          {row.original.source}
        </span>
      ),
    },
    {
      accessorKey: "project_id",
      header: "Project",
      cell: ({ row }) =>
        row.original.project_id ? (
          <Link
            href={`/projects/${encodeURIComponent(row.original.project_id)}`}
            className="text-xs underline decoration-dotted underline-offset-2 hover:text-foreground"
          >
            {projectName(row.original.project_id)}
          </Link>
        ) : (
          <span className="text-xs text-muted-foreground">global</span>
        ),
    },
    {
      accessorKey: "chunk_count",
      header: "Chunks",
      cell: ({ row }) => (
        <Badge variant="outline" className="font-mono text-[10px]">
          {row.original.chunk_count}
        </Badge>
      ),
    },
    {
      accessorKey: "updated_at",
      header: "Updated",
      cell: ({ row }) => (
        <span className="text-xs text-muted-foreground">
          {new Date(row.original.updated_at).toLocaleString()}
        </span>
      ),
    },
    {
      id: "actions",
      header: "",
      cell: ({ row }) => (
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive hover:text-destructive"
          onClick={() => setDeleteTarget(row.original)}
        >
          <Trash2 className="size-3.5" aria-hidden="true" />
        </Button>
      ),
    },
  ];

  const table = useReactTable({
    data: documents.data ?? [],
    columns,
    getCoreRowModel: getCoreRowModel(),
  });

  return (
    <div className="space-y-6 max-w-4xl">
      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Documents</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Reference material ingested via <code className="font-mono">cairn documents
            ingest</code> - scoped to a project when ingested from inside one, global
            otherwise.
          </p>
        </div>
        <HelpButton content={HELP["/documents"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Search chunks</CardTitle>
          <CardDescription>
            Preview what a search query would surface across every visible document.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <div className="flex gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
              <Input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Search document chunks..."
                className="pl-8"
              />
            </div>
            <Select value={projectFilter} onValueChange={setProjectFilter}>
              <SelectTrigger className="w-[160px]">
                <SelectValue placeholder="Project" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL_PROJECTS}>All projects</SelectItem>
                {projects.data?.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {query.length > 0 && (
            <div className="space-y-2">
              {search.isLoading ? (
                <Skeleton className="h-16 w-full" />
              ) : search.data && search.data.length === 0 ? (
                <p className="text-sm text-muted-foreground">No matching chunks.</p>
              ) : (
                search.data?.map((c) => (
                  <div
                    key={c.id}
                    className="rounded-md border border-line/40 bg-muted/30 px-3 py-2 text-xs"
                  >
                    <div className="flex items-center gap-2 mb-1">
                      <span className="font-medium">{c.title}</span>
                      <Badge variant="outline" className="font-mono text-[10px]">
                        #{c.chunk_index}
                      </Badge>
                    </div>
                    <p className="text-muted-foreground line-clamp-2">{c.content}</p>
                  </div>
                ))
              )}
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Ingested documents</CardTitle>
          <CardDescription>
            {documents.data
              ? `${documents.data.length} document(s)${activeProject ? " . scoped to this project + global" : ""}`
              : "Loading..."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {documents.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : documents.data && documents.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              Run <code className="font-mono">cairn documents ingest &lt;path|url&gt;</code> to
              give recall reference material.
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

      <AlertDialog
        open={deleteTarget !== null}
        onOpenChange={(o) => {
          if (!o) setDeleteTarget(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete &quot;{deleteTarget?.title}&quot;?</AlertDialogTitle>
            <AlertDialogDescription>
              Removes {deleteTarget?.chunk_count} chunk(s) from RAG recall. Re-ingest anytime
              with{" "}
              <code className="font-mono">
                cairn documents ingest {deleteTarget?.source}
              </code>
              .
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={del.isPending}
              onClick={() => {
                const target = deleteTarget;
                setDeleteTarget(null);
                if (target) del.mutate(target.id);
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
