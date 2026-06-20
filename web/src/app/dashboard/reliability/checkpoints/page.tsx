"use client";

import { useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { Controller } from "react-hook-form";
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
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Item, ItemContent, ItemTitle, ItemDescription } from "@/components/ui/item";
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
import {
  useCheckpointsQuery,
  useCreateCheckpointMutation,
  useRollbackMutation,
} from "@/lib/queries";
import { checkpointSchema, type CheckpointInput } from "@/lib/forms/schemas";

export default function CheckpointsPage() {
  const checkpoints = useCheckpointsQuery();
  const create = useCreateCheckpointMutation();
  const rollback = useRollbackMutation();
  const [pending, setPending] = useState<string | null>(null);

  const form = useForm<CheckpointInput>({
    resolver: zodResolver(checkpointSchema),
    defaultValues: { label: "" },
  });

  async function onSubmit(values: CheckpointInput) {
    try {
      await create.mutateAsync(values);
      form.reset({ label: "" });
    } catch {
      /* toast handled */
    }
  }

  async function onRollback(id: string) {
    setPending(id);
    try {
      await rollback.mutateAsync(id);
    } finally {
      setPending(null);
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Checkpoints</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Snapshot every file Cairn has tracked, then roll back any tracked file
          to that snapshot.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Create</CardTitle>
        </CardHeader>
        <CardContent>
          <form
            id="form-checkpoint"
            onSubmit={form.handleSubmit(onSubmit)}
            className="flex gap-2"
          >
            <FieldGroup className="flex flex-1 flex-row gap-2">
              <Controller
                name="label"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid} className="flex-1">
                    <FieldLabel htmlFor="form-checkpoint-label" className="sr-only">
                      Label
                    </FieldLabel>
                    <Input
                      {...field}
                      id="form-checkpoint-label"
                      aria-invalid={fieldState.invalid}
                      placeholder="label (optional)"
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Button type="submit" form="form-checkpoint" disabled={create.isPending}>
                {create.isPending ? "…" : "Checkpoint"}
              </Button>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>History</CardTitle>
          <CardDescription>
            Restore tracked files to any snapshot in this list.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {checkpoints.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : checkpoints.data && checkpoints.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No checkpoints — your edits aren't being snapshotted yet. Create
              one before risky changes.
            </p>
          ) : checkpoints.data ? (
            <ul className="space-y-2">
              {checkpoints.data.map((c) => (
                <Item
                  key={c.id}
                  variant="outline"
                  size="sm"
                  className="flex-row items-center justify-between"
                >
                  <ItemContent>
                    <ItemTitle>{c.label || "(unlabeled)"}</ItemTitle>
                    <ItemDescription>
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {c.id.slice(0, 8)}
                      </Badge>
                      {c.files} files ·{" "}
                      {new Date(c.created_at).toLocaleString()}
                    </ItemDescription>
                  </ItemContent>
                  <AlertDialog>
                    <AlertDialogTrigger asChild>
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={pending === c.id}
                      >
                        {pending === c.id ? "…" : "Rollback"}
                      </Button>
                    </AlertDialogTrigger>
                    <AlertDialogContent>
                      <AlertDialogHeader>
                        <AlertDialogTitle>
                          Roll back to {c.id.slice(0, 8)}?
                        </AlertDialogTitle>
                        <AlertDialogDescription>
                          Tracked files on disk will be restored to the snapshot
                          taken at this checkpoint. This is a one-way operation —
                          create a new checkpoint first if you want to be able to
                          undo it.
                        </AlertDialogDescription>
                      </AlertDialogHeader>
                      <AlertDialogFooter>
                        <AlertDialogCancel>Cancel</AlertDialogCancel>
                        <AlertDialogAction onClick={() => onRollback(c.id)}>
                          Roll back
                        </AlertDialogAction>
                      </AlertDialogFooter>
                    </AlertDialogContent>
                  </AlertDialog>
                </Item>
              ))}
            </ul>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}
