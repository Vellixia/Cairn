"use client";

import { useRegistryRevocationsQuery } from "@/lib/queries";
import type { RegistryRevocationEvent } from "@/lib/api";
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

export default function RevocationsContent() {
  const revocations = useRegistryRevocationsQuery();

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>Revocations</CardTitle>
            <HelpButton content={HELP["/registry"]} />
          </div>
        </CardHeader>
        <CardContent>
          {revocations.isLoading ? (
            <Skeleton className="h-48 w-full" />
          ) : !revocations.data || revocations.data.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No revocations yet. Revoked packs appear here so federation peers can
              stay in sync.
            </p>
          ) : (
            <div className="overflow-x-auto rounded-md border border-line">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Pack</TableHead>
                    <TableHead>Version</TableHead>
                    <TableHead>Revoked</TableHead>
                    <TableHead>Reason</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {revocations.data.map((r: RegistryRevocationEvent, i: number) => (
                    <TableRow key={`${r.name}-${r.version}-${i}`}>
                      <TableCell className="font-medium">{r.name}</TableCell>
                      <TableCell>
                        <code className="rounded bg-muted px-1.5 py-0.5 text-[11px] font-mono">
                          {r.version}
                        </code>
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {new Date(r.revoked_at).toLocaleString()}
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {r.reason ?? <span className="italic">No reason given</span>}
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
