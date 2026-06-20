"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useBuildExportMutation } from "@/lib/queries";
import type { ShareExport } from "@/lib/api";
import { toast } from "sonner";

export default function BundlePage() {
  const build = useBuildExportMutation();
  const [bundle, setBundle] = useState<ShareExport | null>(null);

  async function generate() {
    try {
      const r = await build.mutateAsync();
      setBundle(r);
    } catch {
      /* toast handled */
    }
  }

  async function copy() {
    if (!bundle) return;
    await navigator.clipboard.writeText(JSON.stringify(bundle, null, 2));
    toast.success("Bundle copied to clipboard");
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Bundles</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          A sanitized, shareable export of every memory safe to pool with other
          Cairn servers. Imported with{" "}
          <code>cairn-cli import --share bundle.json</code>.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Build</CardTitle>
          <CardDescription>
            Pulls every shareable memory from this server, sanitized.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <Button onClick={generate} disabled={build.isPending}>
            {build.isPending ? "Building…" : "Build shareable bundle"}
          </Button>
          {build.isPending && (
            <div className="space-y-2">
              <Skeleton className="h-4 w-3/4" />
              <Skeleton className="h-4 w-1/2" />
            </div>
          )}
          {bundle && (
            <>
              <dl className="grid grid-cols-2 gap-y-1 text-sm">
                <Stat k="Scanned" v={String(bundle.total)} />
                <Stat k="Shareable" v={String(bundle.shared)} />
                <Stat k="Needs review" v={String(bundle.needs_review)} />
                <Stat k="Withheld (private)" v={String(bundle.withheld)} />
              </dl>
              <Button variant="outline" size="sm" onClick={copy}>
                Copy JSON to clipboard
              </Button>
            </>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function Stat({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between border-b border-dashed border-line py-1">
      <span className="text-muted-foreground">{k}</span>
      <span className="font-mono text-teal">{v}</span>
    </div>
  );
}
