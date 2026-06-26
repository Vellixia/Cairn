"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { Controller } from "react-hook-form";
import {
  type ColumnDef,
  type SortingState,
  flexRender,
  getCoreRowModel,
  getSortedRowModel,
  useReactTable,
} from "@tanstack/react-table";
import { ArrowUpDown, MoreHorizontal, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import {
  useDevicesTokensQuery,
  useIssueTokenMutation,
  useRevokeTokenMutation,
} from "@/lib/queries";
import { issueTokenSchema, type IssueTokenInput } from "@/lib/forms/schemas";
import type { DeviceTokenMeta, IssuedToken } from "@/lib/api";
import { toast } from "sonner";

const SCOPE_VARIANT: Record<string, "default" | "secondary" | "destructive"> = {
  admin: "destructive",
  write: "secondary",
  read: "default",
};

export default function DevicesTokensPage() {
  const tokens = useDevicesTokensQuery();
  const issue = useIssueTokenMutation();
  const revoke = useRevokeTokenMutation();
  const [issued, setIssued] = useState<IssuedToken | null>(null);
  const [pendingRevoke, setPendingRevoke] = useState<string | null>(null);

  const form = useForm<IssueTokenInput>({
    resolver: zodResolver(issueTokenSchema),
    defaultValues: { name: "", scope: "write", expires_in_days: "" },
  });

  async function onSubmit(values: IssueTokenInput) {
    try {
      const t = await issue.mutateAsync(values);
      setIssued(t);
      form.reset({ name: "", scope: "write", expires_in_days: "" });
    } catch {
      /* toast handled */
    }
  }

  async function onRevoke(id: string) {
    setPendingRevoke(id);
    try {
      await revoke.mutateAsync(id);
    } finally {
      setPendingRevoke(null);
    }
  }

  return (
    <div className="space-y-6 max-w-4xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Device tokens</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Issue tokens for CLI / MCP clients to authenticate to this server. The
          bearer is shown once, on issue. Store it like a password.
          </p>
        </div>
        <HelpButton content={HELP["/you/tokens"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Issue a new token</CardTitle>
        </CardHeader>
        <CardContent>
          <form
            id="form-issue-token"
            onSubmit={form.handleSubmit(onSubmit)}
            className="grid gap-3 md:grid-cols-[1fr_8rem_8rem_auto]"
          >
            <FieldGroup className="contents">
              <Controller
                name="name"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-issue-token-name" className="sr-only">
                      Name
                    </FieldLabel>
                    <Input
                      {...field}
                      id="form-issue-token-name"
                      aria-invalid={fieldState.invalid}
                      placeholder="name (e.g. laptop)"
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Controller
                name="scope"
                control={form.control}
                render={({ field }) => (
                  <Field>
                    <FieldLabel htmlFor="form-issue-token-scope" className="sr-only">
                      Scope
                    </FieldLabel>
                    <Select
                      value={field.value}
                      onValueChange={field.onChange}
                    >
                      <SelectTrigger id="form-issue-token-scope" className="w-full">
                        <SelectValue placeholder="scope" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="admin">admin</SelectItem>
                        <SelectItem value="write">write</SelectItem>
                        <SelectItem value="read">read</SelectItem>
                      </SelectContent>
                    </Select>
                  </Field>
                )}
              />
              <Controller
                name="expires_in_days"
                control={form.control}
                render={({ field }) => (
                  <Field>
                    <FieldLabel
                      htmlFor="form-issue-token-expires"
                      className="sr-only"
                    >
                      Days
                    </FieldLabel>
                    <Input
                      id="form-issue-token-expires"
                      type="number"
                      min={1}
                      value={field.value ?? ""}
                      onChange={(e) =>
                        field.onChange(
                          e.target.value === "" ? "" : Math.max(1, parseInt(e.target.value, 10) || 1),
                        )
                      }
                      placeholder="days (no exp)"
                    />
                  </Field>
                )}
              />
              <Button
                type="submit"
                form="form-issue-token"
                disabled={issue.isPending}
              >
                {issue.isPending ? "..." : "Issue"}
              </Button>
            </FieldGroup>
          </form>

          {issued && (
            <div className="mt-4 space-y-2">

              <p className="text-xs text-muted-foreground">
                Copy this token --- it won&apos;t be shown again.
              </p>
              <div className="flex gap-2">

                <code className="flex-1 overflow-x-auto rounded-md border border-primary bg-secondary px-3 py-2 font-mono text-xs text-primary">
                  {issued.token}
                </code>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    navigator.clipboard.writeText(issued.token);
                    toast.success("Copied");
                  }}
                >
                  <Copy /> Copy
                </Button>
              </div>
              <p className="text-[11px] text-muted-foreground">
                On the device:{" "}
                <code className="font-mono">
                  {`cairn sync --server ${typeof window !== "undefined" ? window.location.origin : "<server>"} --token <jwt>`}
                </code>
              </p>
            </div>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Issued tokens</CardTitle>
          <CardDescription>
            Click a row&apos;s action menu to revoke. Future calls using that
            token will return 401.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {tokens.isLoading ? (
            <div className="space-y-2">

              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : tokens.data && tokens.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">No tokens yet. Issue one above.</p>
          ) : tokens.data ? (
            <TokensTable
              data={tokens.data}
              pendingRevoke={pendingRevoke}
              onRevoke={onRevoke}
            />
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}

function TokensTable({
  data,
  pendingRevoke,
  onRevoke,
}: {
  data: DeviceTokenMeta[];
  pendingRevoke: string | null;
  onRevoke: (id: string) => Promise<void>;
}) {
  const [sorting, setSorting] = useState<SortingState>([]);
  const [revokeTarget, setRevokeTarget] = useState<DeviceTokenMeta | null>(null);
  const columns: ColumnDef<DeviceTokenMeta>[] = [
    {
      accessorKey: "name",
      header: ({ column }) => (
        <Button
          variant="ghost"
          size="sm"
          className="-ml-3"
          onClick={() => column.toggleSorting(column.getIsSorted() === "asc")}
        >
          Name
          <ArrowUpDown />
        </Button>
      ),
      cell: ({ row }) => (
        <div>
          <div className="font-medium">{row.original.name}</div>
          <div className="font-mono text-[10px] text-muted-foreground">

            {row.original.id.slice(0, 8)}
          </div>
        </div>
      ),
    },
    {
      accessorKey: "scope",
      header: "Scope",
      cell: ({ row }) => (
        <Badge variant={SCOPE_VARIANT[row.original.scope] ?? "default"} className="font-mono">
          {row.original.scope}
        </Badge>
      ),
    },
    {
      accessorKey: "created_at",
      header: "Created",
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {new Date(row.original.created_at).toLocaleString()}
        </span>
      ),
    },
    {
      accessorKey: "last_used_at",
      header: "Last used",
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {row.original.last_used_at
            ? new Date(row.original.last_used_at).toLocaleString()
            : "---"}
        </span>
      ),
    },
    {
      accessorKey: "expires_at",
      header: "Expires",
      cell: ({ row }) => (
        <span className="text-muted-foreground">
          {row.original.expires_at
            ? new Date(row.original.expires_at).toLocaleString()
            : "never"}
        </span>
      ),
    },
    {
      id: "actions",
      cell: ({ row }) => (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="h-8 w-8"
              aria-label="Open actions"
            >
              <MoreHorizontal />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem
              onSelect={() => {
                navigator.clipboard.writeText(row.original.id);
                toast.success("ID copied");
              }}
            >
              Copy ID
            </DropdownMenuItem>
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              onSelect={() => setRevokeTarget(row.original)}
            >
              Revoke
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      ),
    },
  ];
  const table = useReactTable({
    data,
    columns,
    state: { sorting },
    onSortingChange: setSorting,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
  });
  return (
    <>
      <div className="overflow-x-auto rounded-md border border-line">

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
      <AlertDialog
        open={revokeTarget !== null}
        onOpenChange={(o) => {
          if (!o) setRevokeTarget(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Revoke token {revokeTarget?.id.slice(0, 8)}?
            </AlertDialogTitle>
            <AlertDialogDescription>
              Future calls using this token will return 401. Devices using this
              token will need to re-authenticate.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={pendingRevoke !== null}
              onClick={async () => {
                const target = revokeTarget;
                setRevokeTarget(null);
                if (target) await onRevoke(target.id);
              }}
            >
              {pendingRevoke !== null ? "Revoking..." : "Revoke"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
