"use client";

import { useState } from "react";
import { useQuery, pushToast } from "@/lib/hooks";
import { postJSON, delJSON, type DeviceTokenMeta, type IssuedToken } from "@/lib/api";

export default function DevicesTokensPage() {
  const tokens = useQuery<DeviceTokenMeta[]>("/api/devices/tokens");
  const [name, setName] = useState("");
  const [scope, setScope] = useState<"admin" | "write" | "read">("write");
  const [expires, setExpires] = useState<number | "">("");
  const [issued, setIssued] = useState<IssuedToken | null>(null);
  const [busy, setBusy] = useState(false);

  async function issue() {
    if (!name.trim()) return;
    setBusy(true);
    try {
      const t = await postJSON<IssuedToken>("/api/devices/tokens", {
        name,
        scope,
        expires_in_days: expires === "" ? null : Number(expires),
      });
      setIssued(t);
      setName("");
      tokens.refetch();
      pushToast(`Issued ${t.scope} token for "${t.name}"`, "success");
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Issue failed", "error");
    } finally {
      setBusy(false);
    }
  }

  async function revoke(id: string) {
    if (!window.confirm(`Revoke token ${id.slice(0, 8)}? Future calls using this token will return 401.`)) return;
    try {
      await postJSON(`/api/devices/tokens/${encodeURIComponent(id)}/revoke`, {});
      pushToast("Token revoked", "info");
      tokens.refetch();
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Revoke failed", "error");
    }
  }

  return (
    <div className="space-y-6 max-w-4xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Device tokens</h1>
        <p className="mt-1 text-sm text-slate">
          Issue tokens for CLI / MCP clients to authenticate to this server. The bearer is
          shown once, on issue. Store it like a password.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        <h2 className="text-xs uppercase tracking-[0.08em] text-slate">Issue a new token</h2>
        <div className="grid gap-3 md:grid-cols-[1fr_8rem_8rem_auto]">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="name (e.g. laptop)"
            className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
          />
          <select
            value={scope}
            onChange={(e) => setScope(e.target.value as "admin" | "write" | "read")}
            className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm"
          >
            <option value="admin">admin</option>
            <option value="write">write</option>
            <option value="read">read</option>
          </select>
          <input
            type="number"
            min={1}
            value={expires}
            onChange={(e) => setExpires(e.target.value === "" ? "" : Math.max(1, parseInt(e.target.value, 10) || 1))}
            placeholder="days (no exp)"
            className="rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
          />
          <button
            onClick={issue}
            disabled={busy}
            className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
          >
            {busy ? "…" : "Issue"}
          </button>
        </div>

        {issued && (
          <div className="space-y-2">
            <p className="text-xs text-slate">Copy this token — it won't be shown again.</p>
            <div className="flex gap-2">
              <code className="flex-1 overflow-x-auto rounded-md border border-ember bg-surface2 px-3 py-2 font-mono text-xs text-ember">
                {issued.token}
              </code>
              <button
                onClick={() => navigator.clipboard.writeText(issued.token).then(() => pushToast("Copied", "success"))}
                className="rounded-lg border border-line px-3 py-1.5 text-xs hover:bg-surface2"
              >
                Copy
              </button>
            </div>
            <p className="text-[11px] text-slate">
              On the device:&nbsp;
              <code className="font-mono">{`cairn sync --server ${typeof window !== "undefined" ? window.location.origin : "<server>"} --token <jwt>`}</code>
            </p>
          </div>
        )}
      </section>

      <section className="rounded-xl border border-line bg-surface p-5">
        <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate">Issued tokens</h2>
        {tokens.loading ? (
          <p className="text-sm text-slate">Loading…</p>
        ) : tokens.data && tokens.data.length === 0 ? (
          <p className="text-sm text-slate">No tokens yet. Issue one above.</p>
        ) : tokens.data ? (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b border-line text-left text-[11px] uppercase tracking-wider text-slate">
                  <th className="py-2 pr-4">Name</th>
                  <th className="py-2 pr-4">Scope</th>
                  <th className="py-2 pr-4">Created</th>
                  <th className="py-2 pr-4">Last used</th>
                  <th className="py-2 pr-4">Expires</th>
                  <th className="py-2 pr-4"></th>
                </tr>
              </thead>
              <tbody>
                {tokens.data.map((t) => (
                  <tr key={t.id} className="border-b border-line/60 last:border-0">
                    <td className="py-2 pr-4">
                      <div className="font-medium text-offwhite">{t.name}</div>
                      <div className="text-[10px] font-mono text-slate">{t.id.slice(0, 8)}</div>
                    </td>
                    <td className="py-2 pr-4 font-mono text-teal">{t.scope}</td>
                    <td className="py-2 pr-4 text-slate">{new Date(t.created_at).toLocaleString()}</td>
                    <td className="py-2 pr-4 text-slate">{t.last_used_at ? new Date(t.last_used_at).toLocaleString() : "—"}</td>
                    <td className="py-2 pr-4 text-slate">{t.expires_at ? new Date(t.expires_at).toLocaleString() : "never"}</td>
                    <td className="py-2 pr-4 text-right">
                      <button
                        onClick={() => revoke(t.id)}
                        className="rounded-md border border-line px-2 py-1 text-xs hover:bg-surface2"
                      >
                        Revoke
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </section>
    </div>
  );
}
