"use client";

import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertCircle, Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { CairnMark } from "@/components/logo";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export default function LoginPage() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<unknown>(null);
  const [busy, setBusy] = useState(false);

  // Someone already signed in has no business on this form; a stale bookmark
  // should land them where they were going.
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.me(), retry: false });
  useEffect(() => {
    if (me.data) router.replace("/");
  }, [me.data, router]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.login(email, password);
      // The shell reads `me` from cache; a stale miss would bounce straight
      // back to this page.
      await queryClient.invalidateQueries();
      router.replace("/");
    } catch (err) {
      setError(err);
      setBusy(false);
    }
  }

  const message = error instanceof Error ? error.message : null;

  return (
    <div className="flex min-h-svh items-center justify-center p-6">
      <div className="w-full max-w-sm">
        <div className="mb-6 flex flex-col items-center gap-2 text-center">
          <div className="bg-primary text-primary-foreground flex size-10 items-center justify-center rounded-lg">
            <CairnMark className="size-5" />
          </div>
          <h1 className="text-xl font-semibold tracking-tight">
            Sign in to Cairn
          </h1>
          <p className="text-muted-foreground text-sm text-balance">
            Shared project memory for your coding agents.
          </p>
        </div>

        <Card>
          <CardHeader className="sr-only">
            <CardTitle>Sign in</CardTitle>
            <CardDescription>Use your Cairn account</CardDescription>
          </CardHeader>
          <CardContent>
            <form onSubmit={submit} className="grid gap-4">
              <div className="grid gap-2">
                <Label htmlFor="email">Email</Label>
                <Input
                  id="email"
                  data-testid="email"
                  type="email"
                  autoComplete="username"
                  autoFocus
                  placeholder="you@example.com"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  required
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="password">Password</Label>
                <Input
                  id="password"
                  data-testid="password"
                  type="password"
                  autoComplete="current-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  required
                />
              </div>

              {message && (
                <Alert variant="destructive" role="alert">
                  <AlertCircle />
                  <AlertDescription>{message}</AlertDescription>
                </Alert>
              )}

              <Button
                type="submit"
                data-testid="submit"
                disabled={busy}
                className="w-full"
              >
                {busy && <Loader2 className="animate-spin" />}
                {busy ? "Signing in…" : "Sign in"}
              </Button>
            </form>
          </CardContent>
        </Card>

        <p className="text-muted-foreground mt-6 text-center text-xs text-balance">
          Accounts come from the server&apos;s environment. Ask whoever runs
          this deployment for access.
        </p>
      </div>
    </div>
  );
}
