"use client";

import { Suspense, useEffect } from "react";
import { useRouter, useSearchParams } from "next/navigation";
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
import Logo from "@/components/Logo";
import { ApiError, getJSON, type AuthStatus } from "@/lib/api";
import { useLoginMutation } from "@/lib/queries";
import { loginSchema, type LoginInput } from "@/lib/forms/schemas";

export default function LoginPage() {
  return (
    <Suspense fallback={null}>
      <LoginForm />
    </Suspense>
  );
}

function LoginForm() {
  const router = useRouter();
  const search = useSearchParams();
  const from = search?.get("from") ?? "/dashboard";
  const login = useLoginMutation();

  const form = useForm<LoginInput>({
    resolver: zodResolver(loginSchema),
    defaultValues: { username: "admin", password: "" },
  });

  useEffect(() => {
    getJSON<AuthStatus>("/api/auth/status")
      .then((s) => {
        if (s.setup_required) router.replace("/setup");
      })
      .catch(() => {});
  }, [router]);

  async function onSubmit(values: LoginInput) {
    try {
      await login.mutateAsync(values);
      router.replace(from);
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) {
        form.setError("password", { message: "Invalid username or password." });
      } else if (e instanceof ApiError && e.status === 429) {
        form.setError("root", { message: "Too many attempts. Try again in a minute." });
      } else {
        form.setError("root", {
          message: e instanceof Error ? e.message : "Sign-in failed.",
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
        <Card>
          <CardHeader>
            <CardTitle>Sign in</CardTitle>
            <CardDescription>Dashboard admin account.</CardDescription>
          </CardHeader>
          <CardContent>
            <form
              id="form-login"
              onSubmit={form.handleSubmit(onSubmit)}
              className="space-y-3"
            >
              <FieldGroup>
                <Controller
                  name="username"
                  control={form.control}
                  render={({ field, fieldState }) => (
                    <Field data-invalid={fieldState.invalid}>
                      <FieldLabel htmlFor="form-login-username">Username</FieldLabel>
                      <Input
                        {...field}
                        id="form-login-username"
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
                      <FieldLabel htmlFor="form-login-password">Password</FieldLabel>
                      <Input
                        {...field}
                        id="form-login-password"
                        aria-invalid={fieldState.invalid}
                        type="password"
                        autoComplete="current-password"
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
                    <p
                      role="alert"
                      className="text-sm text-destructive"
                    >
                      {form.formState.errors.root.message}
                    </p>
                  </Field>
                )}
                <Field>
                  <Button
                    type="submit"
                    form="form-login"
                    className="w-full"
                    disabled={login.isPending}
                  >
                    {login.isPending ? "Signing in…" : "Sign in"}
                  </Button>
                </Field>
              </FieldGroup>
            </form>
            <p className="mt-5 text-xs text-muted-foreground">
              Default username <code>admin</code>. First run?{" "}
              <a href="/setup" className="text-primary hover:underline">
                Create admin →
              </a>
            </p>
          </CardContent>
        </Card>
      </div>
    </main>
  );
}
