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
import { Textarea } from "@/components/ui/textarea";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { useSanitizeMutation } from "@/lib/queries";
import { sanitizeSchema, type SanitizeInput } from "@/lib/forms/schemas";
import type { Sanitized, Sensitivity } from "@/lib/api";

const SENSITIVITY: Record<
  Sensitivity,
  { variant: "default" | "destructive"; className?: string }
> = {
  shareable: { variant: "default" },
  needs_review: { variant: "default", className: "border-amber-500/50 text-amber-500 [&>svg]:text-amber-500" },
  private: { variant: "destructive" },
};

export default function SanitizePage() {
  const sanitize = useSanitizeMutation();
  const [result, setResult] = useState<Sanitized | null>(null);
  const form = useForm<SanitizeInput>({
    resolver: zodResolver(sanitizeSchema),
    defaultValues: { text: "" },
  });

  async function onSubmit(values: SanitizeInput) {
    try {
      const r = await sanitize.mutateAsync(values);
      setResult(r);
    } catch {
      /* toast handled */
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Sanitize</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Paste a log line, config snippet, or note. Cairn redacts secrets, emails,
          IPs, and home paths, then classifies the result.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Input</CardTitle>
        </CardHeader>
        <CardContent>
          <form
            id="form-sanitize"
            onSubmit={form.handleSubmit(onSubmit)}
            className="space-y-3"
          >
            <FieldGroup>
              <Controller
                name="text"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-sanitize-text" className="sr-only">
                      Text
                    </FieldLabel>
                    <Textarea
                      {...field}
                      id="form-sanitize-text"
                      aria-invalid={fieldState.invalid}
                      rows={6}
                      className="font-mono"
                      placeholder="Paste anything — a log line, a config snippet, a note."
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Field>
                <Button type="submit" form="form-sanitize" disabled={sanitize.isPending}>
                  {sanitize.isPending ? "…" : "Scan"}
                </Button>
              </Field>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>

      {result && (
        <Card>
          <CardHeader>
            <CardTitle>Result</CardTitle>
            <CardDescription>
              <span className="inline-flex items-center gap-2">
                <Alert
                  variant={SENSITIVITY[result.sensitivity].variant}
                  className={`py-2 ${SENSITIVITY[result.sensitivity].className ?? ""}`}
                >
                  <AlertTitle className="capitalize">
                    {result.sensitivity.replace("_", " ")}
                  </AlertTitle>
                  <AlertDescription>
                    {result.findings.length} redaction
                    {result.findings.length === 1 ? "" : "s"}
                  </AlertDescription>
                </Alert>
              </span>
            </CardDescription>
          </CardHeader>
          <CardContent>
            <ScrollArea className="h-96 rounded-lg border border-line bg-secondary">
              <pre className="whitespace-pre-wrap p-3 font-mono text-xs">
                {result.text}
              </pre>
            </ScrollArea>
            {result.findings.length > 0 && (
              <div className="mt-3 flex flex-wrap gap-1.5">
                {result.findings.map((f, i) => (
                  <Badge key={i} variant="outline" className="font-mono text-[10px]">
                    {f.kind} [{f.start}–{f.end}]
                  </Badge>
                ))}
              </div>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
