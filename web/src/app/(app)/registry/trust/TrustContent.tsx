"use client";

import { Globe, ShieldCheck, Users } from "lucide-react";
import { useRegistryTrustedKeysQuery } from "@/lib/queries";
import type { RegistryTrustGrant } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";

function scopeBadge(allows: string) {
  const map: Record<string, { label: string; icon: typeof Globe }> = {
    public: { label: "Public", icon: Globe },
    team: { label: "Team", icon: Users },
    local: { label: "Local", icon: ShieldCheck },
  };
  const { label, icon: Icon } = map[allows] ?? { label: allows, icon: ShieldCheck };
  return (
    <Badge variant="outline" className="gap-1 font-mono text-[10px]">
      <Icon className="h-3 w-3" />
      {label}
    </Badge>
  );
}

export default function TrustContent() {
  const keys = useRegistryTrustedKeysQuery();

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>Trusted Keys</CardTitle>
            <HelpButton content={HELP["/registry"]} />
          </div>
        </CardHeader>
        <CardContent>
          {keys.isLoading ? (
            <Skeleton className="h-48 w-full" />
          ) : !keys.data || keys.data.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No trusted keys configured. Packs signed by unknown keys will be rejected.
            </p>
          ) : (
            <div className="overflow-x-auto rounded-md border border-line">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Key</TableHead>
                    <TableHead>Scope</TableHead>
                    <TableHead>Label</TableHead>
                    <TableHead>Granted</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {keys.data.map((k: RegistryTrustGrant) => (
                    <TableRow key={k.key}>
                      <TableCell>
                        <code className="rounded bg-muted px-1.5 py-0.5 text-[11px] font-mono">
                          {k.key.slice(0, 16)}…
                        </code>
                      </TableCell>
                      <TableCell>{scopeBadge(k.allows)}</TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {k.label ?? "—"}
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {new Date(k.granted_at).toLocaleDateString()}
                      </TableCell>
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
