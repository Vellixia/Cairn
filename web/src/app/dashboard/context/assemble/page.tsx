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
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { assembleSchema, type AssembleInput } from "@/lib/forms/schemas";
import { getJSON } from "@/lib/api";
import { toast } from "sonner";

interface AssembledItem {
  position: number;
  source: string;
  kind: string;
  content: string;
  score: number;
  est_tokens: number;
}
interface DroppedItem {
  preview: string;
  score: number;
  est_tokens: number;
  reason: string;
}
interface AssemblyReport {
  query: string;
  budget_tokens: number;
  used_tokens: number;
  included: AssembledItem[];
  dropped: DroppedItem[];
  context: string;
}

export default function AssemblePage() {
  const form = useForm<AssembleInput>({
    resolver: zodResolver(assembleSchema),
    defaultValues: { paths: "how does cairn assemble context under a token budget", budget: 2000 },
  });
  const [budget, setBudget] = useState(2000);
  const [report, setReport] = useState<AssemblyReport | null>(null);
  const [busy, setBusy] = useState(false);

  async function onSubmit(values: AssembleInput) {
    setBusy(true);
    try {
      // New /api/context/assemble takes a *query* and a token *budget* (not paths). The form
      // still accepts paths as a convenience: we use them as a single query string by joining
      // with spaces. The "budget" field is the slider's value.
      const q = values.paths.trim();
      const r = await getJSON<AssemblyReport>(
        `/api/context/assemble?q=${encodeURIComponent(q)}&budget=${budget}`,
      );
      setReport(r);
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
          Recall the most relevant memories for a query, edge-order them, pack into a token
          budget. Items that don't fit are reported as dropped — they're always one recall
          away, so nothing is lost.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Inputs</CardTitle>
          <CardDescription>
            A natural-language query and a token budget. Larger budgets include more items.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            id="form-assemble"
            onSubmit={form.handleSubmit(onSubmit)}
            className="space-y-4"
          >
            <FieldGroup>
              <Controller
                name="paths"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-assemble-paths">Query</FieldLabel>
                    <Textarea
                      {...field}
                      id="form-assemble-paths"
                      aria-invalid={fieldState.invalid}
                      rows={2}
                      className="font-mono"
                      placeholder="e.g. how does cairn assemble context under a token budget"
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Field>
                <FieldLabel htmlFor="form-assemble-budget">
                  Token budget: <span className="font-mono">{budget}</span>
                </FieldLabel>
                <input
                  id="form-assemble-budget"
                  type="range"
                  min={500}
                  max={8000}
                  step={250}
                  value={budget}
                  onChange={(e) => setBudget(parseInt(e.target.value, 10))}
                  className="w-full accent-teal"
                />
                <p className="text-xs text-muted-foreground">
                  Drag to resize. Use{" "}
                  <Input
                    type="number"
                    min={100}
                    max={20000}
                    step={100}
                    value={budget}
                    onChange={(e) => setBudget(parseInt(e.target.value, 10) || 0)}
                    className="ml-1 inline-flex h-6 w-24"
                  />{" "}
                  for a precise value.
                </p>
              </Field>
              <Field>
                <Button type="submit" form="form-assemble" disabled={busy}>
                  {busy ? "Assembling…" : "Assemble"}
                </Button>
              </Field>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>

      {busy && <Skeleton className="h-72 w-full" />}

      {report && !busy && (
        <>
          <Card>
            <CardHeader>
              <CardTitle>Result</CardTitle>
              <CardDescription>
                {report.used_tokens} / {report.budget_tokens} tokens used ·{" "}
                {report.included.length} included · {report.dropped.length} dropped
              </CardDescription>
            </CardHeader>
            <CardContent>
              <ScrollArea className="h-72 rounded-lg border border-line bg-secondary">
                <pre className="whitespace-pre-wrap p-3 font-mono text-xs">
                  {report.context}
                </pre>
              </ScrollArea>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Dropped · why</CardTitle>
              <CardDescription>
                Items that didn't fit. Click an item to copy its preview; recall one-by-one
                if you need it back.
              </CardDescription>
            </CardHeader>
            <CardContent>
              {report.dropped.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  Nothing dropped — every relevant memory fit.
                </p>
              ) : (
                <ul className="space-y-2">
                  {report.dropped.map((d, i) => (
                    <li
                      key={i}
                      className="rounded-md border border-line bg-secondary/40 px-3 py-2 text-sm"
                    >
                      <div className="flex items-baseline gap-2">
                        <Badge variant="outline" className="font-mono text-[10px]">
                          {d.reason}
                        </Badge>
                        <span className="font-mono text-[10px] text-muted-foreground">
                          score {d.score.toFixed(3)} · {d.est_tokens} tok
                        </span>
                      </div>
                      <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
                        {d.preview}
                      </p>
                    </li>
                  ))}
                </ul>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}