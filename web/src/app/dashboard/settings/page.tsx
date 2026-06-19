"use client";

import { useRouter } from "next/navigation";
import { useQuery, pushToast } from "@/lib/hooks";
import { type Me } from "@/lib/api";

export default function SettingsPage() {
  const router = useRouter();
  const me = useQuery<Me>("/api/auth/me");

  async function logout() {
    try {
      await fetch("/api/auth/logout", { method: "POST", credentials: "include" });
      pushToast("Signed out", "info");
      router.replace("/login");
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Sign-out failed", "error");
    }
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
        <p className="mt-1 text-sm text-slate">Session info and server connection.</p>
      </header>

      <Section title="Session">
        {me.data ? (
          <dl className="grid grid-cols-2 gap-y-2 text-sm">
            <dt className="text-slate">Username</dt>
            <dd className="font-mono text-offwhite">{me.data.username}</dd>
            <dt className="text-slate">Logged in at</dt>
            <dd className="font-mono text-offwhite">{new Date(me.data.login_at * 1000).toLocaleString()}</dd>
            <dt className="text-slate">Session expires</dt>
            <dd className="font-mono text-offwhite">{new Date(me.data.expires_at * 1000).toLocaleString()}</dd>
            <dt className="text-slate">Generation</dt>
            <dd className="font-mono text-offwhite">{me.data.generation}</dd>
          </dl>
        ) : (
          <p className="text-sm text-slate">Loading…</p>
        )}
        <div className="mt-4 flex gap-2">
          <button
            onClick={logout}
            className="rounded-lg border border-[#f87171] px-3 py-1.5 text-sm font-semibold text-[#f87171] hover:bg-surface2"
          >
            Sign out
          </button>
        </div>
      </Section>

      <Section title="Server">
        <dl className="grid grid-cols-2 gap-y-2 text-sm">
          <dt className="text-slate">API base</dt>
          <dd className="font-mono text-offwhite truncate">{typeof window !== "undefined" ? window.location.origin : "(build-time only)"}</dd>
          <dt className="text-slate">Health endpoint</dt>
          <dd className="font-mono text-offwhite"><code>/api/health</code></dd>
        </dl>
      </Section>

      <Section title="Recovery (loopback-only)">
        <p className="text-sm text-slate">
          Run on the server host:
        </p>
        <pre className="mt-2 rounded-md border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0] overflow-x-auto">{`# Rotate the admin password (bumps generation, invalidates all cookies)
cairn-server admin password

# Delete the admin (next /setup creates a new one)
cairn-server admin reset`}</pre>
        <p className="mt-2 text-xs text-slate">Both refuse on a non-loopback bind.</p>
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-xl border border-line bg-surface p-5">
      <h2 className="mb-3 text-xs uppercase tracking-[0.08em] text-slate">{title}</h2>
      {children}
    </section>
  );
}
