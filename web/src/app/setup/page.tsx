"use client";

// Setup wizard --- two steps the new user walks through on first launch:
//   1. Admin credentials (username + password)
//   2. Health check (database reachable, embedder loaded, admin exists) + finish
//
// Embedding provider is environment-driven only (CAIRN_EMBED_PROVIDER / CAIRN_EMBED_MODEL /
// CAIRN_EMBED_URL / CAIRN_EMBED_API_KEY) --- there is no in-wizard picker. The health step
// surfaces the live, actually-configured provider so the user isn't left guessing. Device
// pairing has been removed entirely; new devices authenticate with a device token instead
// (see the Tokens page under You).

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
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
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { setupSchema, type SetupInput } from "@/lib/forms/schemas";
import { getJSON, postJSON, type AuthStatus } from "@/lib/api";
import { toast } from "sonner";

interface SetupHealth {
  health: {
    db_reachable: boolean;
    admin_exists: boolean;
    embedder_loaded: boolean;
    secret_key_configured: boolean;
  };
  embed_provider: string;
}

export default function SetupPage() {
  const router = useRouter();
  const [step, setStep] = useState<1 | 2>(1);
  const [health, setHealth] = useState<SetupHealth | null>(null);

  // If an admin already exists, this page has nothing left to do - send the visitor to
  // log in instead of showing a "create admin" form the backend would just 409 on submit.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await getJSON<AuthStatus>("/api/auth/status");
        if (!cancelled && !status.setup_required) {
          router.replace("/login");
        }
      } catch {
        // Can't reach the server to check - stay on this page and let submission fail loudly.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [router]);

  const form = useForm<SetupInput>({
    resolver: zodResolver(setupSchema),
    defaultValues: {
      username: "",
      password: "",
      confirm: "",
    },
  });

  async function onSubmit(values: SetupInput) {
    try {
      const res = await postJSON<{ username: string }>("/api/auth/setup", {
        username: values.username,
        password: values.password,
      });
      toast.success(`Welcome, ${res.username}`);
      // Refresh health.
      const h = await getJSON<SetupHealth>("/api/setup/health");
      setHealth(h);
      setStep(2);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "Setup failed");
    }
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <header className="space-y-2">
        <h1 className="text-2xl font-semibold tracking-tight">
          Set up Cairn
        </h1>
        <p className="text-sm text-muted-foreground">
          2 short steps. The dashboard unlocks once setup completes.
        </p>
        <div className="flex gap-1 text-[11px] text-muted-foreground">
          {(["1", "2"] as const).map((label, i) => {
            const n = (i + 1) as 1 | 2;
            const active = step === n;
            const done = step > n;
            return (
              <Badge
                key={label}
                variant={active ? "default" : done ? "secondary" : "outline"}
                className="font-mono"
              >
                {label}
              </Badge>
            );
          })}
        </div>
      </header>

      {step === 1 && (
        <Step1Credentials form={form} onSubmit={form.handleSubmit(onSubmit)} />
      )}
      {step === 2 && (
        <Step2Health
          health={health}
          onContinue={() => router.push("/dashboard")}
        />
      )}
    </div>
  );
}

function Step1Credentials({
  form,
  onSubmit,
}: {
  form: ReturnType<typeof useForm<SetupInput>>;
  onSubmit: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>1. Admin account</CardTitle>
        <CardDescription>
          The single admin who can issue device tokens and review drift events.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form
          id="form-setup-step1"
          onSubmit={onSubmit}
          className="space-y-3"
        >
          <FieldGroup>
            <Controller
              name="username"
              control={form.control}
              render={({ field, fieldState }) => (
                <Field data-invalid={fieldState.invalid}>
                  <FieldLabel htmlFor="form-setup-step1-username">
                    Username
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-setup-step1-username"
                    aria-invalid={fieldState.invalid}
                    autoFocus
                    autoComplete="username"
                  />
                  {fieldState.invalid && (
                    <FieldError errors={[fieldState.error]} />
                  )}
                </Field>
              )}
            />
            <Controller
              name="password"
              control={form.control}
              render={({ field, fieldState }) => (
                <Field data-invalid={fieldState.invalid}>
                  <FieldLabel htmlFor="form-setup-step1-password">
                    Password
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-setup-step1-password"
                    aria-invalid={fieldState.invalid}
                    type="password"
                    autoComplete="new-password"
                  />
                  <FieldDescription>8 characters minimum.</FieldDescription>
                  {fieldState.invalid && (
                    <FieldError errors={[fieldState.error]} />
                  )}
                </Field>
              )}
            />
            <Controller
              name="confirm"
              control={form.control}
              render={({ field, fieldState }) => (
                <Field data-invalid={fieldState.invalid}>
                  <FieldLabel htmlFor="form-setup-step1-confirm">
                    Confirm password
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-setup-step1-confirm"
                    aria-invalid={fieldState.invalid}
                    type="password"
                    autoComplete="new-password"
                  />
                  {fieldState.invalid && (
                    <FieldError errors={[fieldState.error]} />
                  )}
                </Field>
              )}
            />
            <Field>
              <Button type="submit" form="form-setup-step1">
                Create admin
              </Button>
            </Field>
          </FieldGroup>
        </form>
        <p className="mt-3 text-[11px] text-muted-foreground">
          Or set <code>CAIRN_ADMIN_USERNAME</code>,{" "}
          <code>CAIRN_ADMIN_PASSWORD_HASH</code> (Argon2id PHC) in{" "}
          <code>.env</code> and restart.
        </p>
      </CardContent>
    </Card>
  );
}

function Step2Health({
  health,
  onContinue,
}: {
  health: SetupHealth | null;
  onContinue: () => void;
}) {
  if (!health) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>2. Health check</CardTitle>
          <CardDescription>Verifying everything is wired up...</CardDescription>
        </CardHeader>
        <CardContent>
          <Skeleton className="h-32 w-full" />
        </CardContent>
      </Card>
    );
  }
  const allGreen = Object.values(health.health).every(Boolean);
  return (
    <Card>
      <CardHeader>
        <CardTitle>{allGreen ? "All green" : "Almost there"}</CardTitle>
        <CardDescription>2. Health check</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <ul className="space-y-2">
          <Health label="Database reachable" ok={health.health.db_reachable} />
          <Health label="Admin account" ok={health.health.admin_exists} />
          <Health label="Embedder loaded" ok={health.health.embedder_loaded} />
          <Health
            label="Secret key configured"
            ok={health.health.secret_key_configured}
          />
        </ul>
        <p className="text-[11px] text-muted-foreground">
          Embedding provider: <code>{health.embed_provider}</code> (configured via{" "}
          <code>CAIRN_EMBED_PROVIDER</code> --- see docs to change).
        </p>
        <Button onClick={onContinue}>Open dashboard</Button>
      </CardContent>
    </Card>
  );
}

function Health({ label, ok }: { label: string; ok: boolean }) {
  return (
    <li className="flex items-center gap-2 text-sm">
      <span
        className={`inline-block h-2 w-2 rounded-full ${ok ? "bg-emerald-500" : "bg-amber-500"}`}
      />
      <span className={ok ? "" : "text-amber-600"}>{label}</span>
      <Badge variant={ok ? "secondary" : "outline"} className="ml-auto font-mono text-[10px]">
        {ok ? "ok" : "check"}
      </Badge>
    </li>
  );
}
