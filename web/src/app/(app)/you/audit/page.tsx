"use client";

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
import { useDevicesAuditQuery } from "@/lib/queries";
import type { AuditEvent } from "@/lib/api";

function classify(kind: string): "default" | "destructive" | "secondary" {
  if (kind.startsWith("login_failed") || kind === "token_revoked") return "destructive";
  if (kind.startsWith("login_ok") || kind === "setup") return "secondary";
  return "default";
}

export default function AuditPage() {
  const audit = useDevicesAuditQuery();
  const columns: ColumnDef<AuditEvent>[] = [
    {
      accessorKey: "kind",
      header: "Event",
      cell: ({ row }) => (
        <Badge variant={classify(row.original.kind)} className="font-mono text-[10px] uppercase tracking-wider">
          {row.original.kind}
        </Badge>
      ),
    },
    {
      accessorKey: "actor",
      header: "Actor",
      cell: ({ row }) => <span className="text-muted-foreground">{row.original.actor}</span>,
    },
    {
      accessorKey: "detail",
      header: "Detail",
      cell: ({ row }) => <span className="truncate">{row.original.detail}</span>,
    },
    {
      accessorKey: "ts",
      header: "Time",
      cell: ({ row }) => (
        <span className="font-mono text-[11px] text-muted-foreground">
          {new Date(row.original.ts * 1000).toLocaleString()}
        </span>
      ),
    },
  ];
  const table = useReactTable({
    data: audit.data ?? [],
    columns,
    getCoreRowModel: getCoreRowModel(),
  });
  return (
    <div className="space-y-6 max-w-3xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Audit log</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            The last 50 admin events. In-memory only — restart loses it. A
          HelixDB-backed log is a later iteration.
          </p>
        </div>
        <HelpButton content={HELP["/you/audit"]} />
      </header>
      <Card>
        <CardHeader>
          <CardTitle>Events</CardTitle>
          <CardDescription>Polled every 5 seconds.</CardDescription>
        </CardHeader>
        <CardContent>
          {audit.isLoading ? (
            <div className="space-y-2">

              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-full" />
              <Skeleton className="h-6 w-full" />
            </div>
          ) : audit.data && audit.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">No events recorded yet.</p>
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
