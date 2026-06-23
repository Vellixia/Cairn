"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import Link from "next/link";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { Controller } from "react-hook-form";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Field,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemTitle,
} from "@/components/ui/item";
import {
  useDeleteMemoryMutation,
  usePinMemoryMutation,
  useRememberMutation,
  useReinforceMemoryMutation,
} from "@/lib/queries";
import { getJSON } from "@/lib/api";
import { preferSchema, type PreferInput } from "@/lib/forms/schemas";

export default function ProfilePage() {
  const prefs = useQuery({
    queryKey: ["profile", "list"],
    queryFn: () => getJSON<MemoryLite[]>("/api/profile"),
  });
  const remember = useRememberMutation();
  const reinforce = useReinforceMemoryMutation();
  const pin = usePinMemoryMutation();
  const del = useDeleteMemoryMutation();

  const form = useForm<PreferInput>({
    resolver: zodResolver(preferSchema),
    defaultValues: { rule: "" },
  });

  async function onAdd(values: PreferInput) {
    try {
      // Prefer bodies are `{"rule": "..."}` on the API; remember() takes content+kind.
      await remember.mutateAsync({ content: values.rule });
      form.reset({ rule: "" });
      prefs.refetch();
    } catch {
      /* toast handled in mutation */
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Profile</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Standing preferences that every Cairn-backed agent honors. Each one
          has a confidence score — bump it when the agent correctly applied the
          rule, or delete it when the rule no longer applies.
          </p>
        </div>
        <HelpButton content={HELP["/you"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Add a preference</CardTitle>
          <CardDescription>
            Phrase it as a directive ("always use X", "never do Y") so the
            detector picks it up reliably.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            id="form-prefer"
            onSubmit={form.handleSubmit(onAdd)}
            className="space-y-3"
          >
            <FieldGroup>
              <Controller
                name="rule"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-prefer-rule" className="sr-only">
                      Rule
                    </FieldLabel>
                    <Textarea
                      {...field}
                      id="form-prefer-rule"
                      aria-invalid={fieldState.invalid}
                      rows={2}
                      placeholder="e.g. Always use 4-space indentation in Rust"
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Field>
                <Button
                  type="submit"
                  form="form-prefer"
                  disabled={remember.isPending}
                >
                  {remember.isPending ? "Saving…" : "Add preference"}
                </Button>
              </Field>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Active preferences</CardTitle>
          <CardDescription>
            {prefs.data
              ? `${prefs.data.length} stored · sorted newest first`
              : "Loading…"}
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
              No preferences yet. Add one above.
            </p>
          ) : (
            <ul className="space-y-2">
              {prefs.data?.map((p) => (
                <Item key={p.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="line-clamp-2">{p.content}</ItemTitle>
                    <ItemDescription className="flex items-center gap-2">
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
                      disabled={reinforce.isPending}
                      onClick={() => reinforce.mutate(p.id)}
                    >
                      Useful
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={pin.isPending}
                      onClick={() =>
                        pin.mutate({ id: p.id, pinned: !p.pinned })
                      }
                    >
                      {p.pinned ? "Unpin" : "Pin"}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      disabled={del.isPending}
                      onClick={() => del.mutate(p.id)}
                    >
                      Delete
                    </Button>
                  </ItemActions>
                </Item>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <p className="text-[11px] text-muted-foreground">
        See <Link href="/dashboard/memory/wakeup" className="underline">Wakeup</Link> to
        inspect how preferences flow into session bootstrap.
      </p>
    </div>
  );
}

interface MemoryLite {
  id: string;
  kind: string;
  tier: string;
  content: string;
  confidence: number;
  pinned: boolean;
  suspicious: boolean;
  created_at: string;
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