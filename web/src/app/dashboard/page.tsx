"use client";

import { useCallback, useEffect, useState } from "react";
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
  type Checkpoint,
  type RollbackReport,
  type Sanitized,
  type Sensitivity,
  type ShareExport,
} from "@/lib/api";

const TABS = ["Overview", "Memory", "Context", "Reliability", "Preferences", "Share", "Devices"] as const;
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
        <nav className="mb-7 flex flex-wrap gap-1 border-b border-line">
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
        {tab === "Reliability" && <ReliabilityPanel />}
        {tab === "Preferences" && <PreferencesPanel />}
        {tab === "Share" && <SharePanel />}
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

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between border-b border-dashed border-line py-1.5 last:border-0">
      <span className="text-slate">{k}</span>
      <span className="font-mono text-teal">{v}</span>
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

const btnPrimary = "rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206]";
const btnGhost = "rounded-lg border border-line px-4 py-2 text-sm font-semibold hover:border-slate";
const inputCls = "w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-slate";

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
            <Row k="Checkpoints" v={stats?.checkpoints != null ? String(stats.checkpoints) : "…"} />
            <Row k="Preferences" v={stats?.preferences != null ? String(stats.preferences) : "…"} />
            <Row k="Task anchor" v={stats?.anchor ? "set" : "none"} />
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
          {stats?.anchor && (
            <p className="mt-4 text-sm">
              <span className="text-slate">Current task: </span>
              <span className="text-offwhite">{stats.anchor}</span>
            </p>
          )}
          <p className="mt-4 text-sm text-slate">
            Memory, no-loss context, edit guardrails, preference learning, and privacy-first
            sanitization are live. Vectors + graph (HelixDB) and the federated collective-knowledge
            pool are next.
          </p>
        </Card>
      </div>
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
          className={inputCls}
        />
        <button onClick={remember} className={`mt-2 ${btnPrimary}`}>
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
            className={inputCls}
          />
          <button onClick={recall} className={btnGhost}>
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
                {h.memory.concepts?.length > 0 && <> · {h.memory.concepts.join(", ")}</>}
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
  const [mode, setMode] = useState("auto");
  const [result, setResult] = useState<ReadResult | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [err, setErr] = useState("");

  async function read() {
    setExpanded(null);
    try {
      setResult(
        await getJSON<ReadResult>(
          `/api/context/read?path=${encodeURIComponent(path)}&mode=${encodeURIComponent(mode)}`,
        ),
      );
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
    <Card title="Context inspector — cache · AST outline · lossless expand">
      <div className="flex flex-wrap gap-2">
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && read()}
          placeholder="path relative to the server, e.g. crates/cairn-core/src/model.rs"
          className={`${inputCls} flex-1 font-mono`}
        />
        <select
          value={mode}
          onChange={(e) => setMode(e.target.value)}
          className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm"
        >
          <option value="auto">auto</option>
          <option value="full">full</option>
          <option value="signatures">signatures</option>
          <option value="map">map</option>
        </select>
        <button onClick={read} className={btnPrimary}>
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
          <button onClick={expand} className={btnGhost}>
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

function ReliabilityPanel() {
  const [anchor, setAnchor] = useState("");
  const [goal, setGoal] = useState("");
  const [checkpoints, setCheckpoints] = useState<Checkpoint[]>([]);
  const [label, setLabel] = useState("");
  const [note, setNote] = useState("");
  const [err, setErr] = useState("");

  const load = useCallback(async () => {
    try {
      const a = await getJSON<{ anchor: string | null }>("/api/guard/anchor");
      setAnchor(a.anchor ?? "");
      setCheckpoints(await getJSON<Checkpoint[]>("/api/guard/checkpoints"));
      setErr("");
    } catch (e) {
      setErr(String(e));
    }
  }, []);
  useEffect(() => {
    load();
  }, [load]);

  async function setTaskAnchor() {
    if (!goal.trim()) return;
    try {
      await postJSON("/api/guard/anchor", { goal });
      setAnchor(goal);
      setGoal("");
    } catch (e) {
      setErr(String(e));
    }
  }
  async function createCheckpoint() {
    try {
      const q = label.trim() ? `?label=${encodeURIComponent(label.trim())}` : "";
      const cp = await postJSON<Checkpoint>(`/api/guard/checkpoint${q}`, {});
      setNote(`checkpoint ${cp.id.slice(0, 8)} · ${cp.files} files tracked`);
      setLabel("");
      load();
    } catch (e) {
      setErr(String(e));
    }
  }
  async function rollback(id: string) {
    try {
      const r = await postJSON<RollbackReport>(`/api/guard/rollback?id=${encodeURIComponent(id)}`, {});
      setNote(`rolled back ${id.slice(0, 8)} · ${r.restored.length} restored, ${r.skipped.length} skipped`);
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card title="Task anchor — the goal re-injected each session">
        {anchor ? (
          <p className="mb-3 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-offwhite">{anchor}</p>
        ) : (
          <p className="mb-3 text-sm text-slate">No anchor set.</p>
        )}
        <div className="flex gap-2">
          <input
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && setTaskAnchor()}
            placeholder="e.g. Ship the HelixDB backend behind the store seam"
            className={inputCls}
          />
          <button onClick={setTaskAnchor} className={btnPrimary}>
            Set
          </button>
        </div>
      </Card>

      <Card title="Checkpoints — snapshot & roll back tracked files">
        <div className="flex gap-2">
          <input
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && createCheckpoint()}
            placeholder="label (optional)"
            className={inputCls}
          />
          <button onClick={createCheckpoint} className={btnPrimary}>
            Checkpoint
          </button>
        </div>
        {note && <p className="mt-2 text-xs text-teal">{note}</p>}
        {err && <p className="mt-2 text-xs text-ember">{err}</p>}
        <div className="mt-3 space-y-2">
          {checkpoints.length === 0 && <p className="text-sm text-slate">No checkpoints yet.</p>}
          {checkpoints.map((c) => (
            <div
              key={c.id}
              className="flex items-center justify-between rounded-lg border border-line bg-surface2 px-3 py-2 text-sm"
            >
              <div>
                <span className="text-offwhite">{c.label}</span>
                <div className="text-xs text-slate">
                  {c.id.slice(0, 8)} · {c.files} files · {new Date(c.created_at).toLocaleString()}
                </div>
              </div>
              <button onClick={() => rollback(c.id)} className={btnGhost}>
                Rollback
              </button>
            </div>
          ))}
        </div>
      </Card>
    </div>
  );
}

function PreferencesPanel() {
  const [prefs, setPrefs] = useState<Memory[]>([]);
  const [rule, setRule] = useState("");
  const [err, setErr] = useState("");

  const load = useCallback(async () => {
    try {
      setPrefs(await getJSON<Memory[]>("/api/profile"));
      setErr("");
    } catch (e) {
      setErr(String(e));
    }
  }, []);
  useEffect(() => {
    load();
  }, [load]);

  async function add() {
    if (!rule.trim()) return;
    try {
      await postJSON<Memory>("/api/profile", { rule });
      setRule("");
      load();
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <Card title="Preferences — how you like to work, injected into every session">
      <div className="flex gap-2">
        <input
          value={rule}
          onChange={(e) => setRule(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
          placeholder="e.g. always use ripgrep instead of grep"
          className={inputCls}
        />
        <button onClick={add} className={btnPrimary}>
          Add
        </button>
      </div>
      {err && <p className="mt-2 text-xs text-ember">{err}</p>}
      <div className="mt-3 space-y-2">
        {prefs.length === 0 && <p className="text-sm text-slate">No preferences recorded yet.</p>}
        {prefs.map((p) => (
          <div key={p.id} className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm text-offwhite">
            {p.content}
          </div>
        ))}
      </div>
    </Card>
  );
}

function SensitivityBadge({ level }: { level: Sensitivity }) {
  const map: Record<Sensitivity, string> = {
    shareable: "border-teal text-teal",
    needs_review: "border-ember text-ember",
    private: "border-[#f87171] text-[#f87171]",
  };
  return (
    <span className={`rounded-full border px-2.5 py-0.5 text-xs font-semibold ${map[level]}`}>
      {level.replace("_", " ")}
    </span>
  );
}

function SharePanel() {
  const [text, setText] = useState("");
  const [result, setResult] = useState<Sanitized | null>(null);
  const [bundle, setBundle] = useState<ShareExport | null>(null);
  const [err, setErr] = useState("");

  async function scan() {
    if (!text.trim()) return;
    try {
      setResult(await postJSON<Sanitized>("/api/share/sanitize", { text }));
      setErr("");
    } catch (e) {
      setErr(String(e));
    }
  }
  async function exportBundle() {
    try {
      setBundle(await getJSON<ShareExport>("/api/share/export"));
      setErr("");
    } catch (e) {
      setErr(String(e));
    }
  }

  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card title="Sanitize — redact secrets/PII before sharing or logging">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={5}
          placeholder="Paste anything — a log line, a config snippet, a note — and Cairn will redact secrets and classify it."
          className={`${inputCls} font-mono`}
        />
        <button onClick={scan} className={`mt-2 ${btnPrimary}`}>
          Scan
        </button>
        {err && <p className="mt-2 text-xs text-ember">{err}</p>}
        {result && (
          <div className="mt-3 space-y-2">
            <div className="flex items-center gap-2 text-sm">
              <SensitivityBadge level={result.sensitivity} />
              <span className="text-slate">{result.findings.length} redaction(s)</span>
            </div>
            <pre className="max-h-60 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0]">
              {result.text}
            </pre>
          </div>
        )}
      </Card>

      <Card title="Collective knowledge — export a shareable bundle">
        <p className="mb-3 text-sm text-slate">
          Sanitize every memory, withhold anything private, and produce a bundle safe to pool with
          others.
        </p>
        <button onClick={exportBundle} className={btnPrimary}>
          Build shareable bundle
        </button>
        {bundle && (
          <dl className="mt-3 space-y-2 text-sm">
            <Row k="Total scanned" v={String(bundle.total)} />
            <Row k="Shareable" v={String(bundle.shared)} />
            <Row k="Needs review" v={String(bundle.needs_review)} />
            <Row k="Withheld (private)" v={String(bundle.withheld)} />
          </dl>
        )}
        <p className="mt-3 text-xs text-slate">
          On the receiving machine: <span className="font-mono">cairn import --share bundle.json</span>.
        </p>
      </Card>
    </div>
  );
}

function DevicesPanel() {
  return (
    <div className="grid gap-4">
      <Card title="Add a device">
        <p className="mb-3 text-sm text-slate">
          Create a device token on this server, then sync another machine against it:
        </p>
        <code className="block rounded-lg border border-line bg-surface2 px-4 py-3 font-mono text-sm">
          cairn token create my-laptop
        </code>
        <code className="mt-2 block rounded-lg border border-line bg-surface2 px-4 py-3 font-mono text-sm">
          cairn sync --server {API_BASE} --token &lt;token&gt;
        </code>
        <p className="mt-3 text-xs text-slate">
          Last-write-wins. Prefer offline? <span className="font-mono">cairn export dump.json</span>{" "}
          / <span className="font-mono">cairn import dump.json</span>.
        </p>
      </Card>
      <Card title="Connect an agent (MCP)">
        <p className="mb-3 text-sm text-slate">
          Run <span className="font-mono text-ember">cairn install --all</span> to auto-detect and wire
          up every agent (Claude Code, Cursor, VS Code, Windsurf), or add it by hand:
        </p>
        <pre className="rounded-lg border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0]">{`{
  "mcpServers": {
    "cairn": { "command": "cairn", "args": ["mcp"] }
  }
}`}</pre>
      </Card>
    </div>
  );
}
