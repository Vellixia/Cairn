"use client";

import { useQuery } from "@/lib/hooks";
import { type AuditEvent } from "@/lib/api";

export default function AuditPage() {
  const audit = useQuery<AuditEvent[]>("/api/devices/audit", { pollMs: 5_000 });

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Audit log</h1>
        <p className="mt-1 text-sm text-slate">
          The last 50 admin events. In-memory only — restart loses it. A HelixDB-backed log is
          a later iteration.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5">
        {audit.loading ? (
          <p className="text-sm text-slate">Loading…</p>
        ) : audit.data && audit.data.length === 0 ? (
          <p className="text-sm text-slate">No events recorded yet.</p>
        ) : audit.data ? (
          <ul className="space-y-1.5 text-sm">
            {audit.data.map((e, i) => (
              <li key={i} className="flex items-baseline gap-3 rounded-md bg-surface2 px-3 py-1.5">
                <span className={`font-mono text-[11px] uppercase tracking-wider ${
                  e.kind.startsWith("login_failed") || e.kind === "token_revoked"
                    ? "text-[#f87171]"
                    : e.kind.startsWith("login_ok") || e.kind === "setup"
                    ? "text-teal"
                    : "text-ember"
                }`}>
                  {e.kind}
                </span>
                <span className="text-slate">{e.actor}</span>
                <span className="flex-1 text-slate truncate">{e.detail}</span>
                <span className="text-[11px] text-slate">{new Date(e.ts * 1000).toLocaleString()}</span>
              </li>
            ))}
          </ul>
        ) : null}
      </section>
    </div>
  );
}
