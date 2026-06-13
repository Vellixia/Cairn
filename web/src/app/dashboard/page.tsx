"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import Logo from "@/components/Logo";
import {
  API_BASE,
  getJSON,
  postJSON,
  type Health,
  type Stats,
  type ScoredMemory,
  type Memory,
  type ReadResult,
} from "@/lib/api";

const TABS = ["Overview", "Memory", "Context", "Devices"] as const;
type Tab = (typeof TABS)[number];

export default function Dashboard() {
  const [tab, setTab] = useState<Tab>("Overview");
  return (
    <div className="min-h-screen">
      <header className="border-b border-line">
        <div className="mx-auto flex max-w-5xl items-center justify-between px-5 py-4">
          <Link href="/" className="flex items-center gap-2.5">
            <Logo size={26} />
            <span className="font-semibold tracking-tight">Cairn</span>
            <span className="ml-2 rounded-full border border-line px-2 py-0.5 text-xs text-slate">
              control plane
            </span>
          </Link>
          <code className="font-mono text-xs text-slate">{API_BASE}</code>
        </div>
      </header>

      <div className="mx-auto max-w-5xl px-5 py-8">
        <nav className="mb-7 flex gap-1 border-b border-line">
          {TABS.map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`-mb-px border-b-2 px-4 py-2 text-sm ${
                tab === t ? "border-ember text-offwhite" : "border-transparent text-slate hover:text-offwhite"
              }`}
            >
              {t}
            </button>
          ))}
        </nav>

        {tab === "Overview" && <Overview />}
        {tab === "Memory" && <MemoryPanel />}
        {tab === "Context" && <ContextPanel />}
        {tab === "Devices" && <DevicesPanel />}
      </div>
    </div>
  );
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-line bg-surface p-5">
      <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate">{title}</h2>
      {children}
    </div>
  );
}

function OfflineHint() {
  return (
    <p className="text-sm text-slate">
      Can&apos;t reach the server at <code className="font-mono">{API_BASE}</code>. Start it with{" "}
      <code className="font-mono text-ember">cargo run -p cairn-cli -- serve</code>.
    </p>
  );
}

function Overview() {
  const [health, setHealth] = useState<Health | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [offline, setOffline] = useState(false);

  useEffect(() => {
    getJSON<Health>("/api/health")
      .then(setHealth)
      .catch(() => setOffline(true));
    getJSON<Stats>("/api/stats")
      .then(setStats)
      .catch(() => {});
  }, []);

  return (
    <div className="grid gap-4 md:grid-cols-3">
      <Card title="Server">
        {offline ? (
          <OfflineHint />
        ) : (
          <dl className="space-y-2 text-sm">
            <Row k="Status" v={health?.status ?? "…"} />
            <Row k="Version" v={health ? `v${health.version}` : "…"} />
            <Row k="Memories" v={stats ? String(stats.memories) : "…"} />
          </dl>
        )}
      </Card>
      <div className="md:col-span-2">
        <Card title="Pillars">
          <div className="flex flex-wrap gap-2">
            {["Remember", "Compress · no loss", "Assemble lean context", "Stay reliable", "Smarter together"].map(
              (p) => (
                <span key={p} className="rounded-full border border-line bg-surface2 px-3 py-1 text-sm text-[#b9c2cf]">
                  {p}
                </span>
              ),
            )}
          </div>
          <p className="mt-4 text-sm text-slate">
            This is the thin-slice build: Memory and Context are live. Reliability guardrails,
            preference learning, and collective knowledge arrive in later phases.
          </p>
        </Card>
      </div>
    </div>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between border-b border-dashed border-line py-1.5 last:border-0">
      <span className="text-slate">{k}</span>
      <span className="font-mono text-teal">{v}</span>
    </div>
  );
}

function MemoryPanel() {
  const [content, setContent] = useState("");
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<ScoredMemory[]>([]);
  const [note, setNote] = useState("");

  async function remember() {
    if (!content.trim()) return;
    try {
      const m = await postJSON<Memory>("/api/memory", { content });
      setNote(`stored ${m.kind}/${m.tier} · ${m.id.slice(0, 8)}`);
      setContent("");
    } catch (e) {
      setNote(String(e));
    }
  }

  async function recall() {
    try {
      const r = await getJSON<ScoredMemory[]>(`/api/memory/recall?limit=10&q=${encodeURIComponent(query)}`);
      setHits(r);
    } catch (e) {
      setNote(String(e));
    }
  }

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card title="Remember">
        <textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          rows={4}
          placeholder="e.g. We chose SQLite + a content-hash blob store so compression stays lossless."
          className="w-full rounded-lg border border-line bg-surface2 p-3 text-sm outline-none focus:border-slate"
        />
        <button onClick={remember} className="mt-2 rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206]">
          Remember
        </button>
        {note && <p className="mt-2 text-xs text-teal">{note}</p>}
      </Card>

      <Card title="Recall">
        <div className="flex gap-2">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && recall()}
            placeholder="search your memory…"
            className="w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-slate"
          />
          <button onClick={recall} className="rounded-lg border border-line px-4 py-2 text-sm font-semibold">
            Recall
          </button>
        </div>
        <div className="mt-3 space-y-2">
          {hits.length === 0 && <p className="text-sm text-slate">No results yet.</p>}
          {hits.map((h) => (
            <div key={h.memory.id} className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm">
              {h.memory.content}
              <div className="mt-1 text-xs text-slate">
                <span className="text-ember">{h.score.toFixed(2)}</span> · {h.memory.kind} · {h.memory.tier}
              </div>
            </div>
          ))}
        </div>
      </Card>
    </div>
  );
}

function ContextPanel() {
  const [path, setPath] = useState("README.md");
  const [result, setResult] = useState<ReadResult | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [err, setErr] = useState("");

  async function read() {
    setExpanded(null);
    try {
      setResult(await getJSON<ReadResult>(`/api/context/read?path=${encodeURIComponent(path)}`));
      setErr("");
    } catch (e) {
      setErr(String(e));
    }
  }
  async function expand() {
    if (!result) return;
    try {
      const r = await getJSON<{ hash: string; content: string }>(
        `/api/context/expand?hash=${encodeURIComponent(result.hash)}`,
      );
      setExpanded(r.content);
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <Card title="Context inspector — read cache + lossless expand">
      <div className="flex gap-2">
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && read()}
          placeholder="path relative to the server, e.g. README.md"
          className="w-full rounded-lg border border-line bg-surface2 px-3 py-2 font-mono text-sm outline-none focus:border-slate"
        />
        <button onClick={read} className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206]">
          Read
        </button>
      </div>
      {err && <p className="mt-2 text-xs text-ember">{err}</p>}
      {result && (
        <div className="mt-4 space-y-3">
          <div className="flex flex-wrap gap-x-6 gap-y-1 text-sm">
            <Row k="status" v={result.status} />
            <Row k="lines" v={String(result.lines)} />
            <Row k="est. tokens" v={String(result.est_tokens)} />
            <Row k="handle" v={result.handle} />
          </div>
          <p className="text-xs text-slate">{result.note}</p>
          <button onClick={expand} className="rounded-lg border border-line px-4 py-2 text-sm font-semibold">
            Expand → recover byte-identical original
          </button>
          <pre className="max-h-80 overflow-auto rounded-lg border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0]">
            {expanded ?? (result.view || "(cached view — expand to see the full original)")}
          </pre>
        </div>
      )}
    </Card>
  );
}

function DevicesPanel() {
  return (
    <div className="grid gap-4">
      <Card title="Add a device">
        <p className="mb-3 text-sm text-slate">
          Run this on a new machine to install Cairn, pair it to this server, and auto-configure
          every installed agent:
        </p>
        <code className="block rounded-lg border border-line bg-surface2 px-4 py-3 font-mono text-sm">
          curl -fsSL https://cairn.sh/i | sh -s -- pair CAIRN-7Q3X
        </code>
        <p className="mt-3 text-xs text-slate">
          (Pairing + per-device tokens land in Phase 2; this is the documented flow.)
        </p>
      </Card>
      <Card title="Connect an agent (MCP)">
        <p className="mb-3 text-sm text-slate">Point any MCP-capable agent at Cairn:</p>
        <pre className="rounded-lg border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0]">{`{
  "mcpServers": {
    "cairn": { "command": "cairn", "args": ["mcp"] }
  }
}`}</pre>
      </Card>
    </div>
  );
}
