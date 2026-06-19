"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import Logo from "@/components/Logo";
import { ApiError, getJSON, postJSON, type AuthStatus, type Me } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function SetupPage() {
  const router = useRouter();
  const usernameRef = useRef<HTMLInputElement>(null);
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [needsSetup, setNeedsSetup] = useState<boolean | null>(null);

  useEffect(() => {
    usernameRef.current?.focus();
    getJSON<AuthStatus>("/api/auth/status")
      .then((s) => setNeedsSetup(s.setup_required))
      .catch(() => setNeedsSetup(null));
  }, []);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (password.length < 8) {
      setError("Password must be at least 8 characters.");
      return;
    }
    if (password !== confirm) {
      setError("Passwords do not match.");
      return;
    }
    setPending(true);
    try {
      await postJSON("/api/auth/setup", { username, password });
      const me = await getJSON<Me>("/api/auth/me");
      pushToast(`Admin "${me.username}" created`, "success");
      router.replace("/dashboard");
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        setError("An admin already exists. Use the Sign in page instead.");
      } else if (e instanceof ApiError && e.status === 429) {
        setError("Too many attempts. Try again in a minute.");
      } else {
        setError(e instanceof Error ? e.message : "Setup failed.");
      }
    } finally {
      setPending(false);
    }
  }

  return (
    <main className="min-h-screen flex items-center justify-center px-5 py-12">
      <div className="w-full max-w-sm">
        <div className="flex items-center gap-2.5 mb-6 justify-center">
          <Logo size={36} />
          <span className="text-xl font-semibold tracking-tight">Cairn</span>
        </div>

        <div className="rounded-2xl border border-line bg-surface p-6 shadow-lg shadow-black/30">
          <h1 className="text-lg font-semibold">Create admin</h1>
          <p className="mt-1 text-sm text-slate">
            First-run setup. Choose the username and password that will own this dashboard.
          </p>

          {needsSetup === false && (
            <p className="mt-4 rounded-md border border-ember bg-surface2 px-3 py-2 text-sm text-ember">
              An admin already exists — <a className="underline" href="/login">go to sign in</a>.
            </p>
          )}

          <form onSubmit={onSubmit} className="mt-5 space-y-3">
            <label className="block">
              <span className="block text-xs uppercase tracking-wider text-slate mb-1">Username</span>
              <input
                ref={usernameRef}
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoComplete="username"
                required
                className="w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
              />
            </label>
            <label className="block">
              <span className="block text-xs uppercase tracking-wider text-slate mb-1">New password</span>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="new-password"
                required
                minLength={8}
                className="w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
              />
              <span className="mt-1 block text-[11px] text-slate">8+ characters. Hashed with Argon2id.</span>
            </label>
            <label className="block">
              <span className="block text-xs uppercase tracking-wider text-slate mb-1">Confirm</span>
              <input
                type="password"
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
                autoComplete="new-password"
                required
                minLength={8}
                className="w-full rounded-lg border border-line bg-surface2 px-3 py-2 text-sm outline-none focus:border-ember"
              />
            </label>

            {error && (
              <p role="alert" className="rounded-md border border-[#f87171] bg-surface2 px-3 py-2 text-sm text-[#f87171]">
                {error}
              </p>
            )}

            <button
              type="submit"
              disabled={pending}
              className="w-full rounded-lg bg-ember px-4 py-2 font-semibold text-[#1a1206] disabled:opacity-50"
            >
              {pending ? "Creating…" : "Create admin"}
            </button>
          </form>

          <p className="mt-5 text-xs text-slate">
            Or set <code className="font-mono">CAIRN_ADMIN_USERNAME</code>,{" "}
            <code className="font-mono">CAIRN_ADMIN_PASSWORD_HASH</code> (Argon2id PHC) in{" "}
            <code className="font-mono">.env</code> and restart.
          </p>
        </div>
      </div>
    </main>
  );
}
