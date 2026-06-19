"use client";

import { useState } from "react";
import Link from "next/link";
import { useQuery, pushToast } from "@/lib/hooks";
import { type Health, type Stats, type Memory, type AuditEvent } from "@/lib/api";

export default function DashboardOverviewPage() {
  const health = useQuery<Health>("/api/health", { pollMs: 15_000 });
  const stats = useQuery<Stats>("/api/stats", { pollMs: 10_000 });
  const memories = useQuery<Memory[]>("/api/memory/wakeup?limit=5", { pollMs: 30_000 });
  const audit = useQuery<AuditEvent[]>("/api/devices/audit", { pollMs: 30_000 });

  const rel = stats.data?.reliability;
  const scoreColor =
    !rel ? "text-slate" :
    rel.score >= 80 ? "text-teal" :
    rel.score >= 50 ? "text-ember" :
    "text-[#f87171]";

  return (
    <div className="space-y-6">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Overview</h1>
        <p className="mt-1 text-sm text-slate">
          Server health, reliability, recent memory, and the last few admin actions.
        </p>
      </header>

      <section className="grid gap-4 md:grid-cols-3">
        <Card title="Server" loading={health.loading}>
          <Row k="Status" v={health.data?.status ?? (health.error ? "offline" : "…")} />
          <Row k="Version" v={health.data ? `v${health.data.version}` : "…"} />
          <Row k="Memories" v={stats.data ? String(stats.data.memories) : "…"} />
          <Row k="Checkpoints" v={stats.data?.checkpoints != null ? String(stats.data.checkpoints) : "…"} />
          <Row k="Preferences" v={stats.data?.preferences != null ? String(stats.data.preferences) : "…"} />
          <Row k="Anchor" v={stats.data?.anchor ? `"${stats.data.anchor}"` : "none"} />
        </Card>

        <Card title="Reliability" loading={stats.loading}>
          {rel ? (
            <>
              <div className={`text-4xl font-bold ${scoreColor}`}>
                {rel.score}
                <span className="text-base text-slate">/100</span>
              </div>
              <p className="mt-1 text-xs text-slate">
                {rel.samples} edit{rel.samples === 1 ? "" : "s"} ·{" "}
                <span className="text-teal">{rel.ok} ok</span> ·{" "}
                <span className="text-ember">{rel.warn} warn</span> ·{" "}
                <span className="text-[#f87171]">{rel.danger} danger</span> · {rel.rollbacks} rollback{rel.rollbacks === 1 ? "" : "s"}
              </p>
            </>
          ) : (
            <p className="text-sm text-slate">No edit history yet.</p>
          )}
        </Card>

        <Card title="Quick actions">
          <div className="grid grid-cols-2 gap-2">
            <Link href="/dashboard/memory" className={btnCls}>Remember</Link>
            <Link href="/dashboard/memory/recall" className={btnCls}>Recall</Link>
            <Link href="/dashboard/share/sanitize" className={btnCls}>Sanitize</Link>
            <Link href="/dashboard/devices" className={btnCls}>Issue token</Link>
          </div>
          <p className="mt-3 text-[11px] text-slate">
            ⌘K opens the command palette. <kbd className="font-mono">?</kbd> shows shortcuts.
          </p>
        </Card>
      </section>

      <section className="grid gap-4 lg:grid-cols-2">
        <Card title="Recent memory" loading={memories.loading}>
          {memories.data && memories.data.length === 0 && (
            <p className="text-sm text-slate">No memories yet. Try Remember above.</p>
          )}
          {memories.data && memories.data.length > 0 && (
            <ul className="space-y-1.5">
              {memories.data.slice(0, 5).map((m) => (
                <li key={m.id} className="rounded-md bg-surface2 px-3 py-2 text-sm">
                  {m.content}
                  <div className="mt-0.5 text-[11px] text-slate">
                    <span className="text-ember font-mono">{m.kind}</span> · {m.tier} · {new Date(m.created_at).toLocaleString()}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </Card>

        <Card title="Recent admin events" loading={audit.loading}>
          {audit.data && audit.data.length === 0 && (
            <p className="text-sm text-slate">No events recorded yet.</p>
          )}
          {audit.data && audit.data.length > 0 && (
            <ul className="space-y-1.5 text-sm">
              {audit.data.slice(0, 8).map((e, i) => (
                <li key={i} className="flex items-baseline gap-3 rounded-md bg-surface2 px-3 py-1.5">
                  <span className={`font-mono text-[11px] uppercase tracking-wider ${
                    e.kind.startsWith("login_failed") || e.kind === "token_revoked"
                      ? "text-[#f87171]"
                      : "text-teal"
                  }`}>
                    {e.kind}
                  </span>
                  <span className="flex-1 text-slate truncate">{e.detail}</span>
                  <span className="text-[11px] text-slate">{relativeTime(e.ts)}</span>
                </li>
              ))}
            </ul>
          )}
          <p className="mt-2 text-[11px] text-slate">In-memory ring buffer; lost on restart.</p>
        </Card>
      </section>

      <section>
        <Card title="Set a task anchor">
          <AnchorEditor
            current={stats.data?.anchor ?? null}
            onSaved={() => stats.refetch()}
          />
        </Card>
      </section>
    </div>
  );
}

const btnCls =
  "rounded-lg border border-line bg-surface2 px-3 py-2 text-sm hover:bg-surface text-center";

function Card({ title, children, loading }: { title: string; children: React.ReactNode; loading?: boolean }) {
  return (
    <div className="rounded-xl border border-line bg-surface p-5">
      <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate flex items-center gap-2">
        {title}
        {loading && <span className="cairn-skeleton h-3 w-8 inline-block" />}
      </h2>
      {children}
    </div>
  );
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div className="flex justify-between border-b border-dashed border-line py-1.5 text-sm last:border-0">
      <span className="text-slate">{k}</span>
      <span className="font-mono text-teal truncate max-w-[60%]">{v}</span>
    </div>
  );
}

function AnchorEditor({ current, onSaved }: { current: string | null; onSaved: () => void }) {
  const [goal, setGoal] = useState(current ?? "");
  const [busy, setBusy] = useState(false);
  async function save() {
    if (!goal.trim()) return;
    setBusy(true);
    try {
      const { postJSON } = await import("@/lib/api");
      await postJSON("/api/guard/anchor", { goal });
      pushToast("Anchor set", "success");
      onSaved();
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Failed to set anchor", "error");
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="space-y-2">
      {current && (
        <p className="rounded-md border border-line bg-surface2 px-3 py-2 text-sm text-offwhite">{current}</p>
      )}
      <div className="flex gap-2">
        <input
          value={goal}
          onChange={(e) => setGoal(e.target.value)}
          placeholder='e.g. "Ship the HelixDB backend behind the store seam"'
          className="flex-1 rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
        />
        <button
          onClick={save}
          disabled={busy}
          className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
        >
          {current ? "Update" : "Set"}
        </button>
      </div>
    </div>
  );
}

function relativeTime(ts: number): string {
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - ts);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}
