"use client";

import { useState } from "react";
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
import { useRecallQuery } from "@/lib/queries";
import { recallSchema, type RecallInput } from "@/lib/forms/schemas";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { Controller } from "react-hook-form";

export default function RecallPage() {
  const form = useForm<RecallInput>({
    resolver: zodResolver(recallSchema),
    defaultValues: { q: "" },
  });
  const [submitted, setSubmitted] = useState("");
  const query = useRecallQuery(submitted);

  function onSubmit(values: RecallInput) {
    setSubmitted(values.q);
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Recall</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          BM25 lexical recall with semantic fallback when embeddings are enabled.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Search</CardTitle>
          <CardDescription>Submit a query to score memories against.</CardDescription>
        </CardHeader>
        <CardContent>
          <form
            id="form-recall"
            onSubmit={form.handleSubmit(onSubmit)}
            className="flex gap-2"
          >
            <FieldGroup className="flex flex-1 flex-row gap-2">
              <Controller
                name="q"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid} className="flex-1">
                    <FieldLabel htmlFor="form-recall-q" className="sr-only">
                      Query
                    </FieldLabel>
                    <Input
                      {...field}
                      id="form-recall-q"
                      aria-invalid={fieldState.invalid}
                      placeholder='e.g. "why SQLite"'
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Button type="submit" form="form-recall">
                Recall
              </Button>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Results</CardTitle>
        </CardHeader>
        <CardContent>
          {submitted === "" ? (
            <p className="text-sm text-muted-foreground">Search to see results.</p>
          ) : query.isLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : query.data && query.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">No matches yet.</p>
          ) : query.data ? (
            <ul className="space-y-2">
              {query.data.map((h) => (
                <Item key={h.memory.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="line-clamp-3">{h.memory.content}</ItemTitle>
                    <ItemDescription>
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {h.score.toFixed(2)}
                      </Badge>
                      {h.memory.kind} · {h.memory.tier}
                      {h.memory.concepts?.length > 0
                        ? ` · ${h.memory.concepts.join(", ")}`
                        : ""}
                    </ItemDescription>
                  </ItemContent>
                </Item>
              ))}
            </ul>
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}
