"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { useConfigQuery, useLogoutMutation } from "@/lib/queries";
import { useMeStore } from "@/lib/stores/me";
import type { ConfigEntry } from "@/lib/api";

const CONFIG_GROUP_ORDER = ["server", "database", "intelligence", "automation", "guard"];

function configValue(v: unknown): string {
  if (Array.isArray(v)) return v.length > 0 ? v.join(", ") : "(none)";
  if (typeof v === "boolean") return v ? "on" : "off";
  return String(v);
}

export default function SettingsPage() {
  const router = useRouter();
  const me = useMeStore((s) => s.me);
  const logout = useLogoutMutation();
  const config = useConfigQuery();

  const grouped = new Map<string, ConfigEntry[]>();
  for (const e of config.data ?? []) {
    const list = grouped.get(e.group) ?? [];
    list.push(e);
    grouped.set(e.group, list);
  }

  function handleLogout() {
    logout.mutate(undefined, {
      onSettled: () => router.replace("/login"),
    });
  }

  return (
    <div className="space-y-6 max-w-2xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Session info and server connection.
          </p>
        </div>
        <HelpButton content={HELP["/you/settings"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Session</CardTitle>
        </CardHeader>
        <CardContent>
          {me ? (
            <dl className="grid grid-cols-2 gap-y-2 text-sm">
              <dt className="text-muted-foreground">Username</dt>
              <dd className="font-mono">{me.username}</dd>
              <dt className="text-muted-foreground">Logged in at</dt>
              <dd className="font-mono">
                {new Date(me.login_at * 1000).toLocaleString()}
              </dd>
              <dt className="text-muted-foreground">Session expires</dt>
              <dd className="font-mono">
                {new Date(me.expires_at * 1000).toLocaleString()}
              </dd>
              <dt className="text-muted-foreground">Generation</dt>
              <dd className="font-mono">{me.generation}</dd>
            </dl>
          ) : (
            <p className="text-sm text-muted-foreground">Loading...</p>
          )}
          <div className="mt-4 flex gap-2">

            <AlertDialog>
              <AlertDialogTrigger asChild>
                <Button variant="destructive" disabled={logout.isPending}>
                  Sign out
                </Button>
              </AlertDialogTrigger>
              <AlertDialogContent>
                <AlertDialogHeader>
                  <AlertDialogTitle>Sign out of Cairn?</AlertDialogTitle>
                  <AlertDialogDescription>
                    This clears your session cookie on this device. You will need
                    to sign in again to manage this server.
                  </AlertDialogDescription>
                </AlertDialogHeader>
                <AlertDialogFooter>
                  <AlertDialogCancel>Cancel</AlertDialogCancel>
                  <AlertDialogAction onClick={handleLogout}>
                    Sign out
                  </AlertDialogAction>
                </AlertDialogFooter>
              </AlertDialogContent>
            </AlertDialog>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Server</CardTitle>
        </CardHeader>
        <CardContent>
          <dl className="grid grid-cols-2 gap-y-2 text-sm">
            <dt className="text-muted-foreground">API base</dt>
            <dd className="font-mono truncate">
              {typeof window !== "undefined"
                ? window.location.origin
                : "(build-time only)"}
            </dd>
            <dt className="text-muted-foreground">Health endpoint</dt>
            <dd className="font-mono">
              <code>/api/health</code>
            </dd>
          </dl>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Personalization</CardTitle>
          <CardDescription>
            Standing preferences honored by every Cairn-backed agent.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button asChild variant="outline" size="sm">
            <Link href="/you?tab=profile">Open profile</Link>
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Server configuration</CardTitle>
          <CardDescription>
            The effective value of every knob and the env var that changes it. Config is
            read once at boot - edit your <code>.env</code> (or environment) and restart the
            server to apply. Secrets show only set / not set.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          {config.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : (
            CONFIG_GROUP_ORDER.filter((g) => grouped.has(g)).map((group) => (
              <div key={group}>
                <p className="mb-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                  {group}
                </p>
                <ul className="divide-y divide-line/60 rounded-md border border-line">
                  {grouped.get(group)!.map((e) => (
                    <li key={e.key} className="flex items-center gap-3 px-3 py-2 text-xs">
                      <div className="min-w-0 flex-1">
                        <p className="font-mono font-medium">{e.key}</p>
                        <p className="mt-0.5 text-muted-foreground">{e.description}</p>
                      </div>
                      <Badge variant="secondary" className="max-w-[180px] truncate font-mono text-[10px]">
                        {configValue(e.value)}
                      </Badge>
                      <code className="hidden whitespace-nowrap font-mono text-[10px] text-muted-foreground sm:block">
                        {e.env}
                      </code>
                    </li>
                  ))}
                </ul>
              </div>
            ))
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Recovery (env-only bootstrap)</CardTitle>
          <CardDescription>
            Admin credentials are seeded from <code>CAIRN_ADMIN_USERNAME</code> +
            <code className="ml-1">CAIRN_ADMIN_PASSWORD</code> on first boot. To rotate, update
            <code className="ml-1">CAIRN_ADMIN_PASSWORD</code> and restart the container, or wipe
            the data volume (<code>docker compose down -v</code>) to re-bootstrap.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <pre className="rounded-md border border-line bg-secondary p-3 font-mono text-xs overflow-x-auto">{`# Update the password in your .env, then restart:
docker compose up -d cairn

# To reset from scratch (DESTROYS ALL DATA):
docker compose down -v
docker compose up -d cairn`}</pre>
          <p className="mt-2 text-xs text-muted-foreground">
            Both refuse on a non-loopback bind.
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
