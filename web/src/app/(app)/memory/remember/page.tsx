"use client";

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
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Textarea } from "@/components/ui/textarea";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { useRememberMutation } from "@/lib/queries";
import { rememberSchema, type RememberInput } from "@/lib/forms/schemas";

export default function MemoryPage() {
  const remember = useRememberMutation();
  const form = useForm<RememberInput>({
    resolver: zodResolver(rememberSchema),
    defaultValues: { content: "" },
  });
  async function onSubmit(values: RememberInput) {
    try {
      await remember.mutateAsync(values);
      form.reset({ content: "" });
    } catch {
      /* toast handled in mutation */
    }
  }
  return (
    <div className="space-y-6 max-w-3xl">
      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Remember</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Store a memory. Content-hashed, deduped, tiered (working / long-term / archive).
          </p>
        </div>
        <HelpButton content={HELP["/memory"]} />
      </header>
      <Card>
        <CardHeader>
          <CardTitle>New memory</CardTitle>
          <CardDescription>
            Content you want surfaced in future sessions.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            id="form-remember"
            onSubmit={form.handleSubmit(onSubmit)}
            className="space-y-3"
          >
            <FieldGroup>
              <Controller
                name="content"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-remember-content" className="sr-only">
                      Content
                    </FieldLabel>
                    <Textarea
                      {...field}
                      id="form-remember-content"
                      aria-invalid={fieldState.invalid}
                      rows={4}
                      placeholder="e.g. We chose SQLite + a content-hash blob store so compression stays lossless."
                    />
                    <FieldDescription>
                      Press Enter or click Remember. Cairn embeds + indexes immediately.
                    </FieldDescription>
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Field>
                <Button
                  type="submit"
                  form="form-remember"
                  disabled={remember.isPending}
                >
                  {remember.isPending ? "Storing…" : "Remember"}
                </Button>
              </Field>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}