"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
// Pack registry UI (v0.5.0 Sprint 13). Three tabs:
//   Browse   — list + search installed/published packs
//   Publish  — drag-drop a `.cairnpkg` and POST to /registry/packs
//   Revoke   — delete a pack (writes to the append-only revocations log)
//
// The web tier talks to the embedded registry at `/registry/*`; cairn-api gates publish
// against the `trusted_keys.json` set. The publish button shows the Ed25519 verification
// outcome inline so the user can see signed/unsigned before trusting.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
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
import { getJSON, postBinary } from "@/lib/api";

interface PackMeta {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  created_at: string;
  stored_at: string;
  size_bytes: number;
  signer_pubkey: string | null;
  has_ed25519_signature: boolean;
  memory_count: number;
  download_count: number;
  scope?: "local" | "team" | "public";
  origin?: string | null;
  provenance_edge_count?: number;
}

interface ProvenanceManifest {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  created_at: string;
  files: Record<string, string>;
  stats: {
    memories: number;
    profile: number;
    patterns: number;
    graph_edges: number;
  };
  signers: Array<{ /* 32 bytes; rendered as hex on the client */ }>;
}

interface PublishReceipt {
  pack_id: string;
  name: string;
  version: string;
  signed_by: string | null;
  status: "signed" | "unsigned";
  stored_at: string;
}

interface RevocationEvent {
  name: string;
  version: string;
  revoked_at: string;
  reason: string | null;
}

interface TrustedKey {
  // PublicKey is serialized as a 32-byte array in cairn-pack's Manual serde impl.
  // JSON gives us an array of numbers.
  // (We never read this client-side — it's just for completeness.)
}

export default function RegistryPage() {
  const [tab, setTab] = useState<"browse" | "publish" | "revocations">("browse");

  return (
    <div className="space-y-6">

      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Pack registry</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Self-hosted <code>.cairnpkg</code> registry. Packs are signed with Ed25519; install
          verification rejects anything that doesn&apos;t match a key in
          <code> trusted_keys.json</code>.
        </p>
      </header>

      <div
        role="tablist"
        aria-label="Registry sections"
        className="inline-flex rounded-md border border-line bg-muted/40 p-1 text-xs"
      >
        <TabButton current={tab} value="browse" onSelect={setTab}>
          Browse
        </TabButton>
        <TabButton current={tab} value="publish" onSelect={setTab}>
          Publish
        </TabButton>
        <TabButton current={tab} value="revocations" onSelect={setTab}>
          Revocation log
        </TabButton>
      </div>

      {tab === "browse" && <BrowseTab />}
      {tab === "publish" && <PublishTab />}
      {tab === "revocations" && <RevocationsTab />}
    </div>
  );
}

function TabButton<T extends string>({
  current,
  value,
  onSelect,
  children,
}: {
  current: T;
  value: T;
  onSelect: (v: T) => void;
  children: React.ReactNode;
}) {
  const active = current === value;
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      onClick={() => onSelect(value)}
      className={
        "rounded px-3 py-1.5 transition " +
        (active
          ? "bg-background text-foreground shadow-sm"
          : "text-muted-foreground hover:text-foreground")
      }
    >
      {children}
    </button>
  );
}

function BrowseTab() {
  const [q, setQ] = useState("");
  const packs = useQuery({
    queryKey: ["registry", "search", q],
    queryFn: () =>
      getJSON<PackMeta[]>(
        q.trim()
          ? `/registry/search?q=${encodeURIComponent(q.trim())}`
          : "/registry/packs",
      ),
    refetchInterval: 5_000,
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>Published packs</CardTitle>
        <CardDescription>
          Every pack published to <code>/registry/packs</code>. Unsigned packs are stored but
          flagged — install will warn when no trusted signer matches.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex items-center gap-2">
          <Input
            placeholder="Search name / description / author…"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            className="max-w-md"
          />
          {packs.isFetching && (
            <span className="text-xs text-muted-foreground">refreshing…</span>
          )}
        </div>
        {packs.isLoading ? (
          <Skeleton className="h-40 w-full" />
        ) : packs.data && packs.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No packs published yet. Switch to the Publish tab to upload one.
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead className="border-b border-line text-left text-muted-foreground">
                <tr>
                  <th className="py-1 pr-3">Name</th>
                  <th className="py-1 pr-3">Version</th>
                  <th className="py-1 pr-3">Author</th>
                  <th className="py-1 pr-3 text-right">Memories</th>
                  <th className="py-1 pr-3 text-right">Size</th>
                  <th className="py-1 pr-3">Signer</th>
                  <th className="py-1 pr-3">Scope / Prov</th>
                  <th className="py-1 pr-3">Stored</th>
                  <th className="py-1 pr-3">Actions</th>
                </tr>
              </thead>
              <tbody>
                {(packs.data ?? []).map((p) => (
                  <PackRow key={p.id} pack={p} />
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function PackRow({ pack }: { pack: PackMeta }) {
  const qc = useQueryClient();
  const [showProv, setShowProv] = useState(false);
  const revoke = useMutation({
    mutationFn: () =>
      fetch(`/registry/packs/${pack.name}/${pack.version}`, { method: "DELETE" }).then(
        (r) => {
          if (!r.ok) throw new Error(`HTTP ${r.status}`);
        },
      ),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["registry"] });
    },
  });
  return (
    <>
      <tr className="border-b border-line/40">
        <td className="py-2 pr-3 font-mono">{pack.name}</td>
        <td className="py-2 pr-3 font-mono">{pack.version}</td>
        <td className="py-2 pr-3">{pack.author || "—"}</td>
        <td className="py-2 pr-3 text-right font-mono">{pack.memory_count}</td>
        <td className="py-2 pr-3 text-right font-mono">{fmtBytes(pack.size_bytes)}</td>
        <td className="py-2 pr-3">
          {pack.has_ed25519_signature ? (
            <Badge className="bg-emerald-600/20 text-emerald-300 border-emerald-600/40">
              signed
            </Badge>
          ) : (
            <Badge variant="outline" className="text-amber-300 border-amber-600/40">
              unsigned
            </Badge>
          )}
          {pack.signer_pubkey && (
            <span className="ml-2 font-mono text-[10px] text-muted-foreground">
              {pack.signer_pubkey.slice(0, 12)}…
            </span>
          )}
        </td>
        <td className="py-2 pr-3">
          {pack.scope && pack.scope !== "public" && (
            <Badge variant="outline" className="text-[10px]">
              {pack.scope}
            </Badge>
          )}
          {(pack.provenance_edge_count ?? 0) > 0 && (
            <button
              type="button"
              onClick={() => setShowProv((v) => !v)}
              className="ml-2 text-[10px] text-blue-400 hover:underline"
            >
              {pack.provenance_edge_count} edge{pack.provenance_edge_count === 1 ? "" : "s"}
            </button>
          )}
        </td>
        <td className="py-2 pr-3 font-mono text-[10px] text-muted-foreground">
          {new Date(pack.stored_at).toLocaleString()}
        </td>
        <td className="py-2 pr-3">
          <div className="flex gap-2">
            <a
              href={`/registry/packs/${pack.name}/${pack.version}/download`}
              className="text-[10px] text-blue-400 hover:underline"
              download={`${pack.name}-${pack.version}.cairnpkg`}
            >
              download
            </a>
            <button
              type="button"
              onClick={() => revoke.mutate()}
              disabled={revoke.isPending}
              className="text-[10px] text-red-400 hover:underline disabled:opacity-50"
            >
              {revoke.isPending ? "revoking…" : "revoke"}
            </button>
          </div>
        </td>
      </tr>
      {showProv && (
        <tr className="border-b border-line/40 bg-muted/20">
          <td colSpan={8} className="py-3 px-3">
            <ProvenancePanel pack={pack} />
          </td>
        </tr>
      )}
    </>
  );
}

function ProvenancePanel({ pack }: { pack: PackMeta }) {
  const q = useQuery({
    queryKey: ["registry", "manifest", pack.name, pack.version],
    queryFn: () =>
      getJSON<ProvenanceManifest>(
        `/registry/packs/${pack.name}/${pack.version}/manifest.json`,
      ),
    staleTime: 60_000,
  });

  if (q.isLoading) return <Skeleton className="h-24 w-full" />;
  if (q.isError) {
    return (
      <p className="text-xs text-red-400">
        Failed to load manifest: {(q.error as Error).message}
      </p>
    );
  }
  const m = q.data!;
  const files = Object.entries(m.files);
  return (
    <div className="space-y-2 text-xs">
      <div className="flex flex-wrap gap-2">
        <Badge variant="outline">id: {m.id.slice(0, 8)}…</Badge>
        <Badge variant="outline">{m.stats.memories} memories</Badge>
        <Badge variant="outline">{m.stats.profile} preferences</Badge>
        <Badge variant="outline">{m.stats.patterns} patterns</Badge>
        <Badge variant="outline">{m.stats.graph_edges} graph edges</Badge>
      </div>
      <p className="text-muted-foreground">
        Manifest files (sha256 per entry). The graph edges live in{" "}
        <code className="font-mono">graph.jsonl</code> inside the tarball —
        download it to see the full provenance chain.
      </p>
      <div className="overflow-x-auto rounded border border-line bg-background">
        <table className="w-full text-[10px]">
          <thead className="border-b border-line text-muted-foreground">
            <tr>
              <th className="py-1 px-2 text-left">File</th>
              <th className="py-1 px-2 text-left">sha256</th>
            </tr>
          </thead>
          <tbody>
            {files.map(([name, hash]) => (
              <tr key={name} className="border-b border-line/40">
                <td className="py-1 px-2 font-mono">{name}</td>
                <td className="py-1 px-2 font-mono">{hash.slice(0, 16)}…</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function PublishTab() {
  const qc = useQueryClient();
  const [file, setFile] = useState<File | null>(null);
  const [override, setOverride] = useState("");
  const publish = useMutation({
    mutationFn: async (f: File) => {
      const buf = await f.arrayBuffer();
      const qs = override.trim() ? `?trusted=${encodeURIComponent(override.trim())}` : "";
      return postBinary<PublishReceipt>(
        `/registry/packs${qs}`,
        buf,
        "application/x-cairnpkg",
      );
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["registry"] });
      setFile(null);
    },
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>Publish a .cairnpkg</CardTitle>
        <CardDescription>
          Drop a tarball to POST it to <code>/registry/packs</code>. If the tarball carries an
          Ed25519 signature, the registry verifies it against the trusted-keys set (or the
          one-off override below).
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="rounded border border-dashed border-line bg-muted/30 p-6 text-center">
          <input
            type="file"
            accept=".cairnpkg,.ctxpkg,application/x-cairnpkg"
            onChange={(e) => setFile(e.target.files?.[0] ?? null)}
            className="block w-full text-sm text-foreground file:mr-3 file:rounded file:border-0 file:bg-blue-600 file:px-3 file:py-1.5 file:text-white hover:file:bg-blue-500"
          />
          {file && (
            <p className="mt-2 text-xs text-muted-foreground">
              selected: <span className="font-mono">{file.name}</span> ({fmtBytes(file.size)})
            </p>
          )}
        </div>
        <div className="space-y-1.5">
          <label className="text-xs text-muted-foreground">
            Trusted-key override (hex pubkey, optional)
          </label>
          <Input
            placeholder="64-char hex Ed25519 public key"
            value={override}
            onChange={(e) => setOverride(e.target.value)}
            className="max-w-xl font-mono"
          />
          <p className="text-[10px] text-muted-foreground">
            Leave blank to use the registry&apos;s <code>trusted_keys.json</code>. Use the override
            field for a one-off publish under a key you haven&apos;t globally trusted yet.
          </p>
        </div>
        <Button
          disabled={!file || publish.isPending}
          onClick={() => file && publish.mutate(file)}
        >
          {publish.isPending ? "publishing…" : "Publish"}
        </Button>
        {publish.isError && (
          <p className="text-sm text-red-400">
            {(publish.error as Error).message}
          </p>
        )}
        {publish.data && <PublishResult receipt={publish.data} />}
      </CardContent>
    </Card>
  );
}

function PublishResult({ receipt }: { receipt: PublishReceipt }) {
  return (
    <div className="rounded border border-line bg-muted/20 p-4">
      <p className="text-sm">
        Stored <code className="font-mono">{receipt.name}@{receipt.version}</code>{" "}
        {receipt.status === "signed" ? (
          <Badge className="ml-2 bg-emerald-600/20 text-emerald-300 border-emerald-600/40">
            signed
          </Badge>
        ) : (
          <Badge variant="outline" className="ml-2 text-amber-300 border-amber-600/40">
            unsigned
          </Badge>
        )}
      </p>
      <dl className="mt-2 grid grid-cols-[max-content_1fr] gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <dt>pack_id</dt>
        <dd className="font-mono">{receipt.pack_id}</dd>
        {receipt.signed_by && (
          <>
            <dt>signed_by</dt>
            <dd className="font-mono">{receipt.signed_by}</dd>
          </>
        )}
        <dt>stored_at</dt>
        <dd className="font-mono">{new Date(receipt.stored_at).toLocaleString()}</dd>
      </dl>
    </div>
  );
}

function RevocationsTab() {
  const revs = useQuery({
    queryKey: ["registry", "revocations"],
    queryFn: () => getJSON<RevocationEvent[]>("/registry/revocations"),
    refetchInterval: 5_000,
  });

  return (
    <Card>
      <CardHeader>
        <CardTitle>Revocation log</CardTitle>
        <CardDescription>
          Append-only — every <code>DELETE /registry/packs/:name/:version</code> lands here.
          Federation peers replay this file to drop revoked packs on their next sync
          (Sprint 14).
        </CardDescription>
      </CardHeader>
      <CardContent>
        {revs.isLoading ? (
          <Skeleton className="h-40 w-full" />
        ) : revs.data && revs.data.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No revocations yet. Revoking a pack writes one entry.
          </p>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead className="border-b border-line text-left text-muted-foreground">
                <tr>
                  <th className="py-1 pr-3">Name</th>
                  <th className="py-1 pr-3">Version</th>
                  <th className="py-1 pr-3">Revoked at</th>
                </tr>
              </thead>
              <tbody>
                {(revs.data ?? []).map((r, i) => (
                  <tr key={`${r.name}-${r.version}-${i}`} className="border-b border-line/40">
                    <td className="py-1 pr-3 font-mono">{r.name}</td>
                    <td className="py-1 pr-3 font-mono">{r.version}</td>
                    <td className="py-1 pr-3 font-mono text-[10px]">
                      {new Date(r.revoked_at).toLocaleString()}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function fmtBytes(b: number): string {
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KiB`;
  return `${(b / (1024 * 1024)).toFixed(2)} MiB`;
}
