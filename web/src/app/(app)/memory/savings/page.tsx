"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { useQuery } from "@tanstack/react-query";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { getJSON } from "@/lib/api";
import { ArrowLeft } from "lucide-react";
import Link from "next/link";

interface LedgerEntry {
  id: number;
  ts: string;
  source: string;
  bytes_in: number;
  bytes_out: number;
  tokens_saved: number;
  cost_usd_saved: number;
  signature: string;
}

interface MetricsResponse {
  savings: {
    compact_bytes: number;
    full_bytes: number;
    saved_bytes: number;
    saved_ratio: number;
    calls: number;
    hits: number;
    bounces: number;
    hit_rate: number;
    bounce_rate: number;
    wakeup_tokens: number;
    recall_tokens: number;
  };
  usd_saved: number;
  memories: number;
  checkpoints: number;
}

export default function SavingsPage() {
  const metrics = useQuery({
    queryKey: ["metrics"],
    queryFn: () => getJSON<MetricsResponse>("/api/metrics"),
    refetchInterval: 15_000,
  });
  const ledger = useQuery({
    queryKey: ["ledger"],
    queryFn: () => getJSON<LedgerEntry[]>("/api/ledger?limit=200"),
    refetchInterval: 15_000,
  });

  const snap = metrics.data?.savings;
  const cumulative = metrics.data?.usd_saved ?? 0;
  const tokensSaved = (ledger.data ?? []).reduce((a, e) => a + Math.max(0, e.tokens_saved), 0);
  const top = (ledger.data ?? []).slice(0, 12);

  return (
    <div className="space-y-6">

      <header>
        <Link
          href="/memory"
          className="text-xs text-muted-foreground hover:text-foreground inline-flex items-center gap-1"
        >
          <ArrowLeft className="size-3" aria-hidden="true" /> Memory
        </Link>
        <h1 className="mt-1 text-2xl font-semibold tracking-tight">Savings &amp; recover</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Every compact read + assemble pass appends a signed entry. The ledger HMACs each row
          against <code>CAIRN_SECRET_KEY</code> --- <code>/api/ledger/verify</code> re-checks any
          entry offline.
        </p>
      </header>

      <section className="grid gap-4 md:grid-cols-4">
        <Stat
          label="Saved bytes"
          value={snap ? fmtBytes(snap.saved_bytes) : "..."}
        />
        <Stat
          label="Saved ratio"
          value={snap ? `${(snap.saved_ratio * 100).toFixed(1)}%` : "..."}
        />
        <Stat
          label="USD saved (input)"
          value={`$${cumulative.toFixed(4)}`}
        />
        <Stat
          label="Tokens saved (ledger)"
          value={fmtTokens(tokensSaved)}
        />
      </section>

      <section className="grid gap-4 md:grid-cols-3">
        <Stat label="Reads served" value={snap?.calls ?? "..."} />
        <Stat
          label="Hit rate"
          value={snap ? `${(snap.hit_rate * 100).toFixed(0)}%` : "..."}
        />
        <Stat
          label="Bounce rate"
          value={snap ? `${(snap.bounce_rate * 100).toFixed(0)}%` : "..."}
        />
      </section>

      <Card>
        <CardHeader>
          <CardTitle>Recent ledger</CardTitle>
          <CardDescription>
            {ledger.data
              ? `${ledger.data.length} entries . newest first . HMAC-signed`
              : "Loading..."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {ledger.isLoading ? (
            <Skeleton className="h-72 w-full" />
          ) : ledger.data && ledger.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No savings yet --- read a file or run an assemble to start the ledger.
            </p>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-xs">
                <thead className="border-b border-line text-left text-muted-foreground">
                  <tr>
                    <th className="py-1 pr-3">id</th>
                    <th className="py-1 pr-3">ts</th>
                    <th className="py-1 pr-3">source</th>
                    <th className="py-1 pr-3 text-right">bytes in</th>
                    <th className="py-1 pr-3 text-right">bytes out</th>
                    <th className="py-1 pr-3 text-right">tokens saved</th>
                    <th className="py-1 pr-3 text-right">$ saved</th>
                    <th className="py-1 pr-3">signature</th>
                  </tr>
                </thead>
                <tbody>
                  {top.map((e) => (
                    <tr key={e.id} className="border-b border-line/40">
                      <td className="py-1 pr-3 font-mono">#{e.id}</td>
                      <td className="py-1 pr-3 font-mono text-[10px]">
                        {new Date(e.ts).toLocaleString()}
                      </td>
                      <td className="py-1 pr-3">
                        <Badge variant="outline" className="font-mono text-[10px]">
                          {e.source}
                        </Badge>
                      </td>
                      <td className="py-1 pr-3 text-right font-mono">{e.bytes_in}</td>
                      <td className="py-1 pr-3 text-right font-mono">{e.bytes_out}</td>
                      <td className="py-1 pr-3 text-right font-mono">
                        {e.tokens_saved.toLocaleString()}
                      </td>
                      <td className="py-1 pr-3 text-right font-mono">
                        ${e.cost_usd_saved.toFixed(4)}
                      </td>
                      <td className="py-1 pr-3 font-mono text-[10px] text-muted-foreground">
                        {e.signature.slice(0, 16)}...
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string | number }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs uppercase tracking-wider text-muted-foreground font-normal">
          {label}
        </CardTitle>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-semibold font-mono">{value}</div>
      </CardContent>
    </Card>
  );
}

function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KiB`;
  return `${(b / (1024 * 1024)).toFixed(2)} MiB`;
}

function fmtTokens(t: number): string {
  if (t < 1000) return `${t}`;
  return `${(t / 1000).toFixed(1)}k`;
}