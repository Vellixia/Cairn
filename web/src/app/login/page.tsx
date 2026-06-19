"use client";

import { Suspense, useEffect, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import Logo from "@/components/Logo";
import { ApiError, getJSON, postJSON, type AuthStatus, type Me } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

export default function LoginPage() {
  return (
    <Suspense fallback={null}>
      <LoginForm />
    </Suspense>
  );
}

function LoginForm() {
  const router = useRouter();
  const search = useSearchParams();
  const from = search?.get("from") ?? "/dashboard";

  const usernameRef = useRef<HTMLInputElement>(null);
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<AuthStatus | null>(null);

  useEffect(() => {
    usernameRef.current?.focus();
    // If setup is required, redirect so the operator never sees a useless "invalid credentials"
    // after hitting /login by mistake on a brand-new install.
    getJSON<AuthStatus>("/api/auth/status")
      .then((s) => {
        setStatus(s);
        if (s.setup_required) router.replace("/setup");
      })
      .catch(() => {});
  }, [router]);

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    setPending(true);
    try {
      await postJSON("/api/auth/login", { username, password });
      // The cookie is set by the server response; verify it before redirecting.
      const me = await getJSON<Me>("/api/auth/me");
      pushToast(`Welcome back, ${me.username}`, "success");
      router.replace(from);
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        setError("Invalid username or password.");
      } else if (e instanceof ApiError && e.status === 429) {
        setError("Too many attempts. Try again in a minute.");
      } else {
        setError(e instanceof Error ? e.message : "Sign-in failed.");
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
          <h1 className="text-lg font-semibold">Sign in</h1>
          <p className="mt-1 text-sm text-slate">Dashboard admin account.</p>

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
              <span className="block text-xs uppercase tracking-wider text-slate mb-1">Password</span>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="current-password"
                required
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
              {pending ? "Signing in…" : "Sign in"}
            </button>
          </form>

          <p className="mt-5 text-xs text-slate">
            Default username <code>admin</code>. First run?{" "}
            <a href="/setup" className="text-teal hover:underline">Create admin →</a>
          </p>
          {status && !status.setup_required && (
            <p className="mt-1 text-[11px] text-slate">
              Forgot the password? On the server:{" "}
              <code className="font-mono">cairn-server admin password</code> (loopback only).
            </p>
          )}
        </div>
      </div>
    </main>
  );
}
