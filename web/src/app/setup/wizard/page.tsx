"use client";

// v2 setup wizard --- five steps the new user walks through on first launch:
//   1. Admin credentials (username + password)
//   2. Embed provider (default: local hashing; opt into local ONNX or OpenAI-compatible)
//   3. (optional) device pair --- generate a QR code to onboard a phone/tablet
//   4. Green-health check (Helix reachable, embedder loaded, admin exists)
//   5. Done --- drop the user at /dashboard
//
// The existing `/setup` route stays as a v1 fallback (deprecation banner). Both POST to
// `/api/auth/setup`, which now accepts the embed fields.

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import Link from "next/link";
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
import { setupWizardSchema, type SetupWizardInput } from "@/lib/forms/schemas";
import { getJSON, postJSON } from "@/lib/api";
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

interface EmbedDefault {
  provider: string;
  model: string | null;
  url: string | null;
  needs_api_key: boolean;
  description: string;
}

export default function SetupWizardPage() {
  const router = useRouter();
  const [step, setStep] = useState<1 | 2 | 3 | 4>(1);
  const [health, setHealth] = useState<SetupHealth | null>(null);
  const [embedDefault, setEmbedDefault] = useState<EmbedDefault | null>(null);

  useEffect(() => {
    getJSON<EmbedDefault>("/api/setup/embed-default")
      .then(setEmbedDefault)
      .catch(() => setEmbedDefault(null));
  }, []);

  const form = useForm<SetupWizardInput>({
    resolver: zodResolver(setupWizardSchema),
    defaultValues: {
      username: "",
      password: "",
      confirm: "",
      embed_provider: "hashing",
      embed_model: "",
      embed_url: "",
      embed_api_key: "",
    },
  });

  async function onSubmit(values: SetupWizardInput) {
    if (values.password !== values.confirm) {
      form.setError("confirm", { message: "Passwords do not match" });
      return;
    }
    try {
      const res = await postJSON<{ username: string; embed: string | null }>(
        "/api/auth/setup",
        {
          username: values.username,
          password: values.password,
          embed_provider: values.embed_provider,
          embed_model: values.embed_model || null,
          embed_url: values.embed_url || null,
          embed_api_key: values.embed_api_key || null,
        },
      );
      toast.success(`Welcome, ${res.username}`);
      // Refresh health.
      const h = await getJSON<SetupHealth>("/api/setup/health");
      setHealth(h);
      setStep(4);
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
          v2 wizard . 4 short steps. The dashboard unlocks once setup completes.
        </p>
        <div className="flex gap-1 text-[11px] text-muted-foreground">
          {(["1", "2", "3", "4"] as const).map((label, i) => {
            const n = (i + 1) as 1 | 2 | 3 | 4;
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
        <Step1Credentials
          form={form}
          embedDefault={embedDefault}
          onNext={() => setStep(2)}
        />
      )}
      {step === 2 && (
        <Step2Embed
          form={form}
          embedDefault={embedDefault}
          onNext={() => setStep(3)}
          onBack={() => setStep(1)}
        />
      )}
      {step === 3 && (
        <Step3Pair
          onBack={() => setStep(2)}
          onSubmit={form.handleSubmit(onSubmit)}
        />
      )}
      {step === 4 && (
        <Step4Health
          health={health}
          onContinue={() => router.push("/dashboard")}
        />
      )}

      <p className="text-[11px] text-muted-foreground">
        Prefer the old one-step form?{" "}
        <Link href="/setup" className="underline">
          Use v1 setup
        </Link>{" "}
        (deprecated in v0.5.0).
      </p>
    </div>
  );
}

function Step1Credentials({
  form,
  embedDefault,
  onNext,
}: {
  form: ReturnType<typeof useForm<SetupWizardInput>>;
  embedDefault: EmbedDefault | null;
  onNext: () => void;
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
          id="form-wizard-step1"
          onSubmit={form.handleSubmit(() => onNext())}
          className="space-y-3"
        >
          <FieldGroup>
            <Controller
              name="username"
              control={form.control}
              render={({ field, fieldState }) => (
                <Field data-invalid={fieldState.invalid}>
                  <FieldLabel htmlFor="form-wizard-step1-username">
                    Username
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-wizard-step1-username"
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
                  <FieldLabel htmlFor="form-wizard-step1-password">
                    Password
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-wizard-step1-password"
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
                  <FieldLabel htmlFor="form-wizard-step1-confirm">
                    Confirm password
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-wizard-step1-confirm"
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
              <Button type="submit" form="form-wizard-step1">
                Next
              </Button>
            </Field>
          </FieldGroup>
        </form>
        {embedDefault && (
          <p className="mt-3 text-[11px] text-muted-foreground">
            Default embed provider: <code>{embedDefault.provider}</code> .{" "}
            {embedDefault.description}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function Step2Embed({
  form,
  embedDefault,
  onNext,
  onBack,
}: {
  form: ReturnType<typeof useForm<SetupWizardInput>>;
  embedDefault: EmbedDefault | null;
  onNext: () => void;
  onBack: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>2. Embed provider</CardTitle>
        <CardDescription>
          How memory content is turned into vectors. Local hashing needs nothing; ONNX /
          OpenAI give better semantic recall at the cost of a download or an API key.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form
          id="form-wizard-step2"
          onSubmit={form.handleSubmit(() => onNext())}
          className="space-y-3"
        >
          <FieldGroup>
            <Controller
              name="embed_provider"
              control={form.control}
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor="form-wizard-step2-provider">
                    Provider
                  </FieldLabel>
                  <select
                    id="form-wizard-step2-provider"
                    {...field}
                    className="rounded-md border border-line bg-background px-3 py-2 text-sm"
                  >
                    <option value="hashing">
                      Local hashing (default --- no model, no network)
                    </option>
                    <option value="local">
                      Local ONNX (all-MiniLM-L6-v2, ~90 MB download)
                    </option>
                    <option value="ollama">
                      Ollama (e.g. nomic-embed-text on localhost:11434)
                    </option>
                    <option value="openai">
                      OpenAI-compatible (text-embedding-3-small)
                    </option>
                  </select>
                  <FieldDescription>
                    Stored at <code>embed_config</code> in the meta store; the runtime picks
                    it up at next start.
                  </FieldDescription>
                </Field>
              )}
            />
            <Controller
              name="embed_url"
              control={form.control}
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor="form-wizard-step2-url">
                    URL (Ollama / OpenAI-compatible)
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-wizard-step2-url"
                    placeholder="http://localhost:11434"
                  />
                </Field>
              )}
            />
            <Controller
              name="embed_model"
              control={form.control}
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor="form-wizard-step2-model">
                    Model (optional)
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-wizard-step2-model"
                    placeholder="nomic-embed-text / text-embedding-3-small"
                  />
                </Field>
              )}
            />
            <Controller
              name="embed_api_key"
              control={form.control}
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor="form-wizard-step2-key">
                    API key (OpenAI-compatible only)
                  </FieldLabel>
                  <Input
                    {...field}
                    id="form-wizard-step2-key"
                    type="password"
                    autoComplete="off"
                  />
                </Field>
              )}
            />
            <div className="flex gap-2">
              <Button type="button" variant="outline" onClick={onBack}>
                Back
              </Button>
              <Button type="submit" form="form-wizard-step2">
                Next
              </Button>
            </div>
          </FieldGroup>
        </form>
        {embedDefault && (
          <p className="mt-3 text-[11px] text-muted-foreground">
            The wizard pre-selects <code>{embedDefault.provider}</code>. Override above if
            you have an ONNX model ready or an OpenAI key handy.
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function Step3Pair({
  onBack,
  onSubmit,
}: {
  onBack: () => void;
  onSubmit: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>3. Optional device pair</CardTitle>
        <CardDescription>
          Pair a phone or tablet now, or skip and onboard later from the Devices page.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          After setup, visit <code>/dashboard/devices/pair</code> to mint a pairing code and
          scan it with the Cairn CLI on the new device.
        </p>
        <div className="flex gap-2">
          <Button type="button" variant="outline" onClick={onBack}>
            Back
          </Button>
          <Button type="button" onClick={onSubmit}>
            Skip &amp; finish setup
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function Step4Health({
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
          <CardTitle>4. Health check</CardTitle>
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
        <CardDescription>
          Embed provider: <code>{health.embed_provider}</code>
        </CardDescription>
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