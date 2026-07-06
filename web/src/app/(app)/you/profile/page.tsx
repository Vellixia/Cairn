"use client";

import { useState } from "react";
import Link from "next/link";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemTitle,
} from "@/components/ui/item";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Trash2 } from "lucide-react";
import {
  useAddPreferenceMutation,
  useDeletePreferenceMutation,
  usePreferencesQuery,
} from "@/lib/queries";
import type { Memory } from "@/lib/api";

// Web redesign v2 follow-up: preferences were read-only with no explanation of what they were
// for. This is the one profile surface where manual add/delete makes sense - a standing
// preference is a deliberate, rare, one-time-ish decision (a config-style entry), not
// something an agent writes constantly like a memory.
export default function ProfilePage() {
  const prefs = usePreferencesQuery();
  const add = useAddPreferenceMutation();
  const del = useDeletePreferenceMutation();
  const [rule, setRule] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<Memory | null>(null);

  function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    const trimmed = rule.trim();
    if (!trimmed) return;
    add.mutate(trimmed, { onSuccess: () => setRule("") });
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Preferences</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Standing rules every Cairn-backed agent loads at session start - before wakeup
          memories, before anything else. Agents record them automatically with the{" "}
          <code>prefer</code> tool when you say things like &quot;always use ripgrep&quot; or
          &quot;keep responses terse&quot;; add or remove them here for anything you want to set
          once and have every future session honor.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Add a preference</CardTitle>
          <CardDescription>
            Phrase it as a standing instruction - it&apos;s injected verbatim into every
            session.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={onSubmit} className="flex gap-2">
            <Input
              value={rule}
              onChange={(e) => setRule(e.target.value)}
              placeholder="Always use ripgrep instead of grep"
            />
            <Button type="submit" disabled={!rule.trim() || add.isPending}>
              Add
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Active preferences</CardTitle>
          <CardDescription>
            {prefs.data
              ? `${prefs.data.length} stored . sorted newest first . injected at every session start`
              : "Loading..."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {prefs.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : prefs.data && prefs.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No preferences yet. Add one above, or let an agent record one with the{" "}
              <code>prefer</code> tool.
            </p>
          ) : (
            <ul className="space-y-2">
              {prefs.data?.map((p) => (
                <Item key={p.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="line-clamp-2">{p.content}</ItemTitle>
                    <ItemDescription className="flex items-center gap-2 flex-wrap">
                      <Badge
                        variant="outline"
                        className="font-mono text-[10px] uppercase tracking-wider"
                      >
                        {p.kind}
                      </Badge>
                      <ConfidenceBar value={p.confidence} />
                      <span className="font-mono text-[10px] text-muted-foreground">
                        conf {p.confidence.toFixed(2)}
                      </span>
                      {p.pinned && (
                        <Badge variant="secondary" className="text-[10px]">
                          pinned
                        </Badge>
                      )}
                      {p.suspicious && (
                        <Badge variant="destructive" className="text-[10px]">
                          suspicious
                        </Badge>
                      )}
                    </ItemDescription>
                  </ItemContent>
                  <ItemActions>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="text-destructive hover:text-destructive"
                      onClick={() => setDeleteTarget(p)}
                    >
                      <Trash2 className="size-3.5" aria-hidden="true" />
                    </Button>
                  </ItemActions>
                </Item>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <p className="text-[11px] text-muted-foreground">
        Use the{" "}
        <Link href="/memory" className="underline">
          memory browser&apos;s Wakeup-order toggle
        </Link>{" "}
        to see preferences alongside the rest of session bootstrap.
      </p>

      <AlertDialog open={deleteTarget !== null} onOpenChange={(o) => !o && setDeleteTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete this preference?</AlertDialogTitle>
            <AlertDialogDescription>
              &quot;{deleteTarget?.content}&quot; will no longer be injected into future
              sessions.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              disabled={del.isPending}
              onClick={() => {
                const target = deleteTarget;
                setDeleteTarget(null);
                if (target) del.mutate(target.id);
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function ConfidenceBar({ value }: { value: number }) {
  const pct = Math.max(0, Math.min(100, value * 100));
  const color =
    pct >= 80
      ? "bg-emerald-500"
      : pct >= 50
        ? "bg-amber-500"
        : "bg-destructive";
  return (
    <span className="inline-block h-1.5 w-16 overflow-hidden rounded bg-muted">
      <span
        className={`block h-full ${color}`}
        style={{ width: `${pct}%` }}
        aria-label={`confidence ${pct.toFixed(0)}%`}
      />
    </span>
  );
}
