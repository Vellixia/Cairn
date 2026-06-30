"use client";

import { useQuery } from "@tanstack/react-query";
import { Download, Trash2, Globe, ShieldCheck, Users } from "lucide-react";
import Link from "next/link";
import { qk, useRevokePackMutation } from "@/lib/queries";
import { getJSON } from "@/lib/api";
import type { RegistryPackMeta } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";

function scopeLabel(scope: string) {
  const map: Record<string, { label: string; icon: typeof Globe }> = {
    public: { label: "Public", icon: Globe },
    team: { label: "Team", icon: Users },
    local: { label: "Local", icon: ShieldCheck },
  };
  const { label, icon: Icon } = map[scope] ?? { label: scope, icon: ShieldCheck };
  return (
    <span className="inline-flex items-center gap-1 text-sm">
      <Icon className="h-3.5 w-3.5" />
      {label}
    </span>
  );
}

function formatSize(bytes: number | undefined): string {
  if (bytes === undefined || bytes === null) return "—";
  if (bytes > 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
  return `${(bytes / 1024).toFixed(0)} KiB`;
}

export default function PackDetail({ name }: { name: string }) {
  const versions = useQuery({
    queryKey: qk.registryPack(name),
    queryFn: () =>
      getJSON<RegistryPackMeta[]>(`/api/registry/packs/${encodeURIComponent(name)}`),
    enabled: name !== "new",
  });

  const revoke = useRevokePackMutation();
  const latest = versions.data?.[0];

  if (name === "new") {
    return (
      <Card className="p-6 text-center text-sm text-muted-foreground">
        Select a pack from the list.
      </Card>
    );
  }

  if (versions.isLoading) {
    return (
      <div className="space-y-4">
        <Skeleton className="h-32 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (versions.isError || !versions.data || versions.data.length === 0) {
    return (
      <Card className="p-6 text-center text-sm text-muted-foreground">
        Pack &ldquo;{name}&rdquo; not found.
      </Card>
    );
  }

  const metadataRows = [
    { label: "Author", value: latest?.author },
    { label: "Description", value: latest?.description },
    { label: "Scope", value: latest ? scopeLabel(latest.scope ?? "public") : null },
    { label: "Origin", value: latest?.origin },
    {
      label: "Signature",
      value: latest?.has_ed25519_signature ? (
        <Badge>Signed by {latest.signer_pubkey?.slice(0, 16)}…</Badge>
      ) : (
        <Badge variant="secondary">Unsigned</Badge>
      ),
    },
    {
      label: "Provenance edges",
      value: String(latest?.provenance_edge_count ?? 0),
    },
    {
      label: "Total downloads",
      value: String(latest?.download_count ?? 0),
    },
  ];

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="text-xl">{name}</CardTitle>
              {latest?.description && (
                <p className="mt-1 text-sm text-muted-foreground">
                  {latest.description}
                </p>
              )}
            </div>
            <HelpButton content={HELP["/registry"]} />
          </div>
        </CardHeader>
        <CardContent>
          <dl className="grid grid-cols-2 gap-x-8 gap-y-2 text-sm">
            {metadataRows.map(({ label, value }) => (
              <div key={label} className="contents">
                <dt className="font-medium text-muted-foreground">{label}</dt>
                <dd>{value ?? <span className="text-muted-foreground">—</span>}</dd>
              </div>
            ))}
          </dl>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Versions</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="overflow-x-auto rounded-md border border-line">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Version</TableHead>
                  <TableHead>Size</TableHead>
                  <TableHead>Memory count</TableHead>
                  <TableHead>Published</TableHead>
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {versions.data.map((v) => (
                  <TableRow key={v.version}>
                    <TableCell>
                      <code className="rounded bg-muted px-1.5 py-0.5 text-[11px] font-mono">
                        {v.version}
                      </code>
                    </TableCell>
                    <TableCell className="text-sm tabular-nums text-muted-foreground">
                      {formatSize(v.size_bytes)}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {v.memory_count}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {v.stored_at ? new Date(v.stored_at).toLocaleDateString() : "—"}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button variant="ghost" size="icon" asChild>
                          <Link
                            href={`/api/registry/packs/${encodeURIComponent(name)}/${encodeURIComponent(v.version)}/download`}
                          >
                            <Download className="h-4 w-4" />
                          </Link>
                        </Button>
                        <AlertDialog>
                          <AlertDialogTrigger asChild>
                            <Button variant="ghost" size="icon">
                              <Trash2 className="h-4 w-4 text-destructive" />
                            </Button>
                          </AlertDialogTrigger>
                          <AlertDialogContent>
                            <AlertDialogHeader>
                              <AlertDialogTitle>Revoke pack</AlertDialogTitle>
                              <AlertDialogDescription>
                                This will permanently remove {name} v{v.version} and
                                append a revocation event. Federation peers will see
                                this change on their next sync. This action cannot
                                be undone.
                              </AlertDialogDescription>
                            </AlertDialogHeader>
                            <AlertDialogFooter>
                              <AlertDialogCancel>Cancel</AlertDialogCancel>
                              <AlertDialogAction
                                onClick={() =>
                                  revoke.mutate({ name, version: v.version })
                                }
                              >
                                Revoke
                              </AlertDialogAction>
                            </AlertDialogFooter>
                          </AlertDialogContent>
                        </AlertDialog>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
