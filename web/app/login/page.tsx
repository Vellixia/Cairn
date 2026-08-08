"use client";

import { useRouter } from "next/navigation";
import { useState } from "react";
import { api } from "@/lib/api";
import { Card, ErrorNote, PageHeader } from "@/components/ui";

export default function LoginPage() {
  const router = useRouter();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.login(email, password);
      router.push("/");
      router.refresh();
    } catch (err) {
      setError(err);
    } finally {
      setBusy(false);
    }
  }

  return (
    <main>
      <PageHeader title="Sign in" subtitle="Cairn shared project memory" />
      <Card className="max-w-sm">
        <form onSubmit={submit} className="space-y-3">
          <label className="block text-sm">
            Email
            <input
              data-testid="email"
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              className="mt-1 w-full rounded border border-neutral-300 px-2 py-1.5 dark:border-neutral-700 dark:bg-neutral-950"
              required
            />
          </label>
          <label className="block text-sm">
            Password
            <input
              data-testid="password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="mt-1 w-full rounded border border-neutral-300 px-2 py-1.5 dark:border-neutral-700 dark:bg-neutral-950"
              required
            />
          </label>
          <button
            data-testid="submit"
            type="submit"
            disabled={busy}
            className="w-full rounded bg-neutral-900 px-3 py-1.5 text-sm text-white disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900"
          >
            {busy ? "Signing in…" : "Sign in"}
          </button>
          {error != null && <ErrorNote error={error} />}
        </form>
      </Card>
    </main>
  );
}
