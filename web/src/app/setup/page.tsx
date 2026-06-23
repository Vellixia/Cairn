"use client";

import Link from "next/link";
import { useEffect } from "react";
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
  AlertDialog,
  AlertDialogAction,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  Field,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from "@/components/ui/field";
import { Input } from "@/components/ui/input";
import Logo from "@/components/Logo";
import { ApiError, getJSON, type AuthStatus } from "@/lib/api";
import { useSetupMutation } from "@/lib/queries";
import { setupSchema, type SetupInput } from "@/lib/forms/schemas";

export default function SetupPage() {
  const router = useRouter();
  const setup = useSetupMutation();

  const form = useForm<SetupInput>({
    resolver: zodResolver(setupSchema),
    defaultValues: { username: "admin", password: "", confirm: "" },
  });

  useEffect(() => {
    getJSON<AuthStatus>("/api/auth/status")
      .then((s) => {
        if (!s.setup_required) router.replace("/login");
      })
      .catch(() => {});
  }, [router]);

  async function onSubmit(values: SetupInput) {
    try {
      await setup.mutateAsync({ username: values.username, password: values.password });
      router.replace("/dashboard");
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        form.setError("root", {
          message: "An admin already exists. Use the Sign in page instead.",
        });
      } else if (e instanceof ApiError && e.status === 429) {
        form.setError("root", { message: "Too many attempts. Try again in a minute." });
      } else {
        form.setError("root", {
          message: e instanceof Error ? e.message : "Setup failed.",
        });
      }
    }
  }

return (
    <main className="min-h-screen flex items-center justify-center px-5 py-12">
      <div className="w-full max-w-sm">
        <div className="flex items-center gap-2.5 mb-6 justify-center">
          <Logo size={36} />
          <span className="text-xl font-semibold tracking-tight">Cairn</span>
        </div>

        <AlertDialog defaultOpen>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>v1 setup — deprecated in v0.5.0</AlertDialogTitle>
              <AlertDialogDescription>
                The new wizard (admin → embed → pair → health) lives at{" "}
                <Link href="/setup/wizard" className="underline font-mono">
                  /setup/wizard
                </Link>
                . This form still works but skips the embed provider picker.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <Button asChild variant="outline">
                <Link href="/setup/wizard">Open wizard</Link>
              </Button>
              <AlertDialogAction>Continue with v1</AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>

        <Card>
          <CardHeader>
            <CardTitle>Create admin</CardTitle>
            <CardDescription>
              First-run setup. Choose the username and password that will own this
              dashboard.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <form
              id="form-setup"
              onSubmit={form.handleSubmit(onSubmit)}
              className="space-y-3"
            >
              <FieldGroup>
                <Controller
                  name="username"
                  control={form.control}
                  render={({ field, fieldState }) => (
                    <Field data-invalid={fieldState.invalid}>
                      <FieldLabel htmlFor="form-setup-username">Username</FieldLabel>
                      <Input
                        {...field}
                        id="form-setup-username"
                        aria-invalid={fieldState.invalid}
                        autoComplete="username"
                        required
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
                      <FieldLabel htmlFor="form-setup-password">New password</FieldLabel>
                      <Input
                        {...field}
                        id="form-setup-password"
                        aria-invalid={fieldState.invalid}
                        type="password"
                        autoComplete="new-password"
                        required
                      />
                      <FieldDescription>
                        8+ characters. Hashed with Argon2id.
                      </FieldDescription>
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
                      <FieldLabel htmlFor="form-setup-confirm">Confirm</FieldLabel>
                      <Input
                        {...field}
                        id="form-setup-confirm"
                        aria-invalid={fieldState.invalid}
                        type="password"
                        autoComplete="new-password"
                        required
                      />
                      {fieldState.invalid && (
                        <FieldError errors={[fieldState.error]} />
                      )}
                    </Field>
                  )}
                />
                {form.formState.errors.root && (
                  <Field>
                    <p role="alert" className="text-sm text-destructive">
                      {form.formState.errors.root.message}
                    </p>
                  </Field>
                )}
                <Field>
                  <Button
                    type="submit"
                    form="form-setup"
                    className="w-full"
                    disabled={setup.isPending}
                  >
                    {setup.isPending ? "Creating…" : "Create admin"}
                  </Button>
                </Field>
              </FieldGroup>
            </form>
            <p className="mt-5 text-xs text-muted-foreground">
              Or set <code>CAIRN_ADMIN_USERNAME</code>,{" "}
              <code>CAIRN_ADMIN_PASSWORD_HASH</code> (Argon2id PHC) in{" "}
              <code>.env</code> and restart.
            </p>
          </CardContent>
        </Card>
      </div>
    </main>
  );
}
