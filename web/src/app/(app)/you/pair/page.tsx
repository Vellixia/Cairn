"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { Controller } from "react-hook-form";
import { Copy } from "lucide-react";
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
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { useGeneratePairCodeMutation } from "@/lib/queries";
import { pairCodeSchema, type PairCodeInput } from "@/lib/forms/schemas";
import type { PairCode } from "@/lib/api";
import { toast } from "sonner";

export default function PairCodePage() {
  const generate = useGeneratePairCodeMutation();
  const [pair, setPair] = useState<PairCode | null>(null);
  const form = useForm<PairCodeInput>({
    resolver: zodResolver(pairCodeSchema),
    defaultValues: { name: "", ttl_minutes: 10 },
  });

  async function onSubmit(values: PairCodeInput) {
    try {
      const p = await generate.mutateAsync(values);
      setPair(p);
    } catch {
      /* toast handled */
    }
  }

  return (
    <div className="space-y-6 max-w-2xl">

      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Pair a new device</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Generate a short code, then on the new device run{" "}
          <code className="font-mono">
            cairn pair &lt;code&gt; --server {typeof window !== "undefined" ? window.location.origin : "<server>"}
          </code>
          . No long tokens to copy.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Generate</CardTitle>
          <CardDescription>
            TTL: 1--60 minutes (default 10).
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form
            id="form-pair"
            onSubmit={form.handleSubmit(onSubmit)}
            className="grid gap-3 md:grid-cols-[1fr_8rem_auto]"
          >
            <FieldGroup className="contents">
              <Controller
                name="name"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-pair-name" className="sr-only">
                      Device name
                    </FieldLabel>
                    <Input
                      {...field}
                      id="form-pair-name"
                      aria-invalid={fieldState.invalid}
                      placeholder="device name (e.g. laptop)"
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Controller
                name="ttl_minutes"
                control={form.control}
                render={({ field, fieldState }) => (
                  <Field data-invalid={fieldState.invalid}>
                    <FieldLabel htmlFor="form-pair-ttl" className="sr-only">
                      TTL minutes
                    </FieldLabel>
                    <Input
                      id="form-pair-ttl"
                      type="number"
                      min={1}
                      max={60}
                      value={field.value}
                      onChange={(e) => field.onChange(e.target.value)}
                    />
                    {fieldState.invalid && (
                      <FieldError errors={[fieldState.error]} />
                    )}
                  </Field>
                )}
              />
              <Button type="submit" form="form-pair" disabled={generate.isPending}>
                {generate.isPending ? "..." : "Generate"}
              </Button>
            </FieldGroup>
          </form>
        </CardContent>
      </Card>

      {pair && (
        <Alert>
          <AlertTitle>Pair code</AlertTitle>
          <AlertDescription>
            <div className="mt-2 text-center">

              <div className="font-mono text-4xl font-bold tracking-[0.3em] text-primary">

                {pair.code}
              </div>
              <div className="mt-2 text-xs text-muted-foreground">

                valid until {new Date(pair.expires_at).toLocaleString()} . single use
              </div>
              <Button
                variant="outline"
                size="sm"
                className="mt-3"
                onClick={() => {
                  navigator.clipboard.writeText(pair.code);
                  toast.success("Copied");
                }}
              >
                <Copy /> Copy code
              </Button>
            </div>
          </AlertDescription>
        </Alert>
      )}
    </div>
  );
}
