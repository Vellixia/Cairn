"use client";

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
import { useLogoutMutation } from "@/lib/queries";
import { useMeStore } from "@/lib/stores/me";

export default function SettingsPage() {
  const router = useRouter();
  const me = useMeStore((s) => s.me);
  const logout = useLogoutMutation();

  function handleLogout() {
    logout.mutate(undefined, {
      onSettled: () => router.replace("/login"),
    });
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Session info and server connection.
        </p>
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
            <p className="text-sm text-muted-foreground">Loading…</p>
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
          <CardTitle>Recovery (loopback-only)</CardTitle>
          <CardDescription>
            Run on the server host to rotate the password or reset the admin.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <pre className="rounded-md border border-line bg-secondary p-3 font-mono text-xs overflow-x-auto">{`# Rotate the admin password (bumps generation, invalidates all cookies)
cairn-server admin password

# Delete the admin (next /setup creates a new one)
cairn-server admin reset`}</pre>
          <p className="mt-2 text-xs text-muted-foreground">
            Both refuse on a non-loopback bind.
          </p>
        </CardContent>
      </Card>
    </div>
  );
}
