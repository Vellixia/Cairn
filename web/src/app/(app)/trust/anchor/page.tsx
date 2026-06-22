"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
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
import { useAnchorQuery, useSetAnchorMutation } from "@/lib/queries";
import { anchorSchema, type AnchorInput } from "@/lib/forms/schemas";

export default function AnchorPage() {
  const anchor = useAnchorQuery();
  const setAnchor = useSetAnchorMutation();
  const form = useForm<AnchorInput>({
    resolver: zodResolver(anchorSchema),
    defaultValues: { goal: anchor.data?.anchor ?? "" },
  });
  async function onSubmit(values: AnchorInput) {
    try {
      await setAnchor.mutateAsync(values);
      form.reset({ goal: values.goal });
      anchor.refetch();
    } catch {
      /* toast handled */
    }
  }
  return (
    <div className="space-y-6 max-w-2xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Task anchor</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            The goal re-injected at every session start. If you set one, you stop
          having to re-explain the task every time.
          </p>
        </div>
        <HelpButton content={HELP["/trust/anchor"]} />
      </header>
      <Card>
        <CardHeader>
          <CardTitle>Current anchor</CardTitle>
        </CardHeader>
        <CardContent>
          {anchor.data?.anchor ? (
            <p className="rounded-md border border-line bg-secondary px-3 py-2 text-sm">
              {anchor.data.anchor}
            </p>
          ) : (
            <p className="text-sm text-muted-foreground">No anchor set.</p>
          )}
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>Set or update</CardTitle>
          <CardDescription>Plain text, no formatting.</CardDescription>
        </CardHeader>
        <CardContent>
          <form
            id="form-anchor-page"
            onSubmit={form.handleSubmit(onSubmit)}
            className="flex gap-2"
          >
            <FieldGroup className="flex flex-1 flex-row gap-2">
              <Controller
                name="goal"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid} className="flex-1">
                    <FieldLabel htmlFor="form-anchor-page-goal" className="sr-only">
                      Goal
                    </FieldLabel>
                    <Input
                      {...field}
                      id="form-anchor-page-goal"
                      aria-invalid={fieldState.invalid}
                      placeholder='e.g. "Ship the HelixDB backend behind the store seam"'
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Button type="submit" form="form-anchor-page" disabled={setAnchor.isPending}>
                {anchor.data?.anchor ? "Update" : "Set"}
              </Button>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
