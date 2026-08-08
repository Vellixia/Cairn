import { AlertCircle, Inbox } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Skeleton } from "@/components/ui/skeleton";

export function PageHeader({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: string;
  children?: React.ReactNode;
}) {
  return (
    <header className="mb-6 flex flex-wrap items-end justify-between gap-3">
      <div className="min-w-0">
        {/* The title truncates because a project name can be arbitrarily long
            and belongs on one line. The subtitle wraps instead: on a phone it
            is the only place some of this context appears. */}
        <h1 className="truncate text-2xl font-semibold tracking-tight">
          {title}
        </h1>
        {subtitle && (
          <p className="text-muted-foreground mt-1 text-sm text-pretty">
            {subtitle}
          </p>
        )}
      </div>
      {children}
    </header>
  );
}

/** An empty result is a state worth designing, not a blank area. */
export function EmptyState({
  title,
  description,
  children,
}: {
  title: string;
  description?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center rounded-lg border border-dashed px-6 py-14 text-center">
      <Inbox className="text-muted-foreground mb-3 size-8" strokeWidth={1.5} />
      <p className="font-medium">{title}</p>
      {description && (
        <div className="text-muted-foreground mt-1 max-w-sm text-sm text-pretty">
          {description}
        </div>
      )}
      {children && <div className="mt-4">{children}</div>}
    </div>
  );
}

export function ErrorState({ error }: { error: unknown }) {
  const message = error instanceof Error ? error.message : String(error);
  return (
    <Alert variant="destructive" role="alert">
      <AlertCircle />
      <AlertTitle>Something went wrong</AlertTitle>
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}

export function ListSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="space-y-2">
      {Array.from({ length: rows }, (_, i) => (
        <Skeleton key={i} className="h-16 w-full" />
      ))}
    </div>
  );
}

/** Dates render as text the moment they are read, not as a raw timestamp. */
export function formatDate(value: string | null | undefined): string {
  if (!value) return "never";
  return new Date(value).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}
