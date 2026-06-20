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
import { ScrollArea } from "@/components/ui/scroll-area";
import { Textarea } from "@/components/ui/textarea";
import { assembleSchema, type AssembleInput } from "@/lib/forms/schemas";
import { postJSON } from "@/lib/api";
import { toast } from "sonner";

export default function AssemblePage() {
  const form = useForm<AssembleInput>({
    resolver: zodResolver(assembleSchema),
    defaultValues: { paths: "crates/cairn-core/src/lib.rs crates/cairn-api/src/lib.rs README.md", budget: 4000 },
  });
  const [view, setView] = useState<string | null>(null);
  const [report, setReport] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function onSubmit(values: AssembleInput) {
    const paths = values.paths.split(/\s+/).filter(Boolean);
    if (paths.length === 0) {
      toast.error("Add at least one path.");
      return;
    }
    setBusy(true);
    try {
      const qs = paths
        .map((p) => `path=${encodeURIComponent(p)}`)
        .join("&");
      const r = await postJSON<{ view: string; report?: unknown }>(
        `/api/context/assemble?${qs}&budget=${values.budget}`,
        {},
      );
      setView(r.view);
      setReport(r.report ? JSON.stringify(r.report, null, 2) : null);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Assemble failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6 max-w-4xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Assemble</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Pack several files into a token budget. Edge-ordered, reports dropped items.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Inputs</CardTitle>
          <CardDescription>
            One path per token (whitespace-separated).
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            id="form-assemble"
            onSubmit={form.handleSubmit(onSubmit)}
            className="space-y-3"
          >
            <FieldGroup>
              <Controller
                name="paths"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-assemble-paths">Paths</FieldLabel>
                    <Textarea
                      {...field}
                      id="form-assemble-paths"
                      aria-invalid={fieldState.invalid}
                      rows={3}
                      className="font-mono"
                      placeholder="crates/cairn-core/src/lib.rs crates/cairn-api/src/lib.rs README.md"
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Controller
                name="budget"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-assemble-budget">Budget (tokens)</FieldLabel>
                    <Input
                      {...field}
                      id="form-assemble-budget"
                      aria-invalid={fieldState.invalid}
                      type="number"
                      className="w-36"
                      onChange={(e) => field.onChange(e.target.value)}
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Field>
                <Button type="submit" form="form-assemble" disabled={busy}>
                  {busy ? "Assembling…" : "Assemble"}
                </Button>
              </Field>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>

      {view && (
        <Card>
          <CardHeader>
            <CardTitle>Output</CardTitle>
            <CardDescription>
              Files included up to the token budget, with dropped files reported.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-2">
            <ScrollArea className="h-[28rem] rounded-lg border border-line bg-secondary">
              <pre className="whitespace-pre-wrap p-3 font-mono text-xs">{view}</pre>
            </ScrollArea>
            {report && (
              <pre className="rounded-md border border-line bg-secondary p-2 font-mono text-[11px] text-muted-foreground overflow-x-auto">
                {report}
              </pre>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
