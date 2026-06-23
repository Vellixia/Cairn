"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
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
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { getJSON } from "@/lib/api";
import { useContextReadQuery } from "@/lib/queries";
import { contextReadSchema, type ContextReadInput } from "@/lib/forms/schemas";

export default function ContextInspectorPage() {
  const form = useForm<ContextReadInput>({
    resolver: zodResolver(contextReadSchema),
    defaultValues: { path: "README.md", mode: "auto" },
  });
  const [submitted, setSubmitted] = useState<ContextReadInput | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const query = useContextReadQuery(submitted);

  function onSubmit(values: ContextReadInput) {
    setSubmitted(values);
    setExpanded(null);
  }

  async function expand() {
    if (!query.data) return;
    try {
      const r = await getJSON<{ content: string }>(
        `/api/context/expand?hash=${encodeURIComponent(query.data.hash)}`,
      );
      setExpanded(r.content);
    } catch {
      /* toast via api  */
    }
  }

  return (
    <div className="space-y-6 max-w-4xl">

      <header className="flex items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Context Inspector</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Read a file with cache hit, AST outline, or full content — and recover
          the byte-identical original on demand.
          </p>
        </div>
        <HelpButton content={HELP["/memory/inspector"]} />
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Read</CardTitle>
          <CardDescription>
            Pick a path and a mode. Auto picks full or signatures based on file size.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            id="form-context"
            onSubmit={form.handleSubmit(onSubmit)}
            className="flex flex-wrap gap-2"
          >
            <FieldGroup className="flex flex-1 flex-row gap-2 flex-wrap">
              <Controller
                name="path"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid} className="flex-1 min-w-[20rem]">
                    <FieldLabel htmlFor="form-context-path" className="sr-only">
                      Path
                    </FieldLabel>
                    <Input
                      {...field}
                      id="form-context-path"
                      aria-invalid={fieldState.invalid}
                      className="font-mono"
                      placeholder="path relative to the server, e.g. crates/cairn-core/src/model.rs"
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Controller
                name="mode"
                control={form.control}
                render={({ field }) => (
                  <Field>
                    <FieldLabel htmlFor="form-context-mode" className="sr-only">
                      Mode
                    </FieldLabel>
                    <Select
                      value={field.value}
                      onValueChange={field.onChange}
                    >
                      <SelectTrigger id="form-context-mode" className="w-36">
                        <SelectValue placeholder="auto" />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="auto">auto</SelectItem>
                        <SelectItem value="full">full</SelectItem>
                        <SelectItem value="signatures">signatures</SelectItem>
                        <SelectItem value="map">map</SelectItem>
                      </SelectContent>
                    </Select>
                  </Field>
                )}
              />
              <Button type="submit" form="form-context">
                Read
              </Button>
            </FieldGroup>
          </form>

          {query.isLoading ? (
            <div className="mt-4 space-y-2">

              <Skeleton className="h-4 w-3/4" />
              <Skeleton className="h-40 w-full" />
            </div>
          ) : query.data ? (
            <div className="mt-4 space-y-3">

              <div className="grid grid-cols-2 gap-y-1 text-sm md:grid-cols-4">

                <Stat k="status" v={query.data.status} />
                <Stat k="lines" v={String(query.data.lines)} />
                <Stat k="est. tokens" v={String(query.data.est_tokens)} />
                <Stat k="handle" v={query.data.handle.slice(0, 12) + "…"} />
              </div>
              <p className="text-xs text-muted-foreground">{query.data.note}</p>
              <Button variant="outline" size="sm" onClick={expand}>
                Expand → recover byte-identical original
              </Button>
              <ScrollArea className="h-96 rounded-lg border border-line bg-secondary">
                <pre className="p-3 font-mono text-xs">
                  {expanded ??
                    (query.data.view ||
                      "(cached view — expand to see the full original)")}
                </pre>
              </ScrollArea>
            </div>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}

function Stat({ k, v }: { k: string; v: string }) {
  return (
    <div className="rounded-md bg-secondary px-3 py-1.5">

      <div className="text-[10px] uppercase tracking-wider text-muted-foreground">

        {k}
      </div>
      <div className="font-mono text-teal truncate">{v}</div>
    </div>
  );
}
