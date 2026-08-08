import Link from "next/link";

export function Card({
  children,
  className = "",
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={`rounded-lg border border-neutral-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900 ${className}`}
    >
      {children}
    </div>
  );
}

export function Badge({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded-full border border-neutral-300 px-2 py-0.5 text-xs uppercase tracking-wide text-neutral-600 dark:border-neutral-700 dark:text-neutral-400">
      {children}
    </span>
  );
}

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
      <div>
        <h1 className="text-2xl font-semibold">{title}</h1>
        {subtitle && (
          <p className="mt-1 text-sm text-neutral-500">{subtitle}</p>
        )}
      </div>
      {children}
    </header>
  );
}

export function ProjectNav({ id, active }: { id: string; active: string }) {
  const items = [
    { href: `/projects/${id}`, label: "Overview", key: "overview" },
    { href: `/projects/${id}/tasks`, label: "Tasks", key: "tasks" },
    { href: `/projects/${id}/sessions`, label: "Sessions", key: "sessions" },
    { href: `/projects/${id}/memory`, label: "Memory", key: "memory" },
    { href: `/projects/${id}/sync`, label: "Sync", key: "sync" },
  ];
  return (
    <nav className="mb-6 flex flex-wrap gap-1 border-b border-neutral-200 dark:border-neutral-800">
      {items.map((i) => (
        <Link
          key={i.key}
          href={i.href}
          data-testid={`nav-${i.key}`}
          className={`-mb-px border-b-2 px-3 py-2 text-sm ${
            active === i.key
              ? "border-neutral-900 font-medium dark:border-neutral-100"
              : "border-transparent text-neutral-500 hover:text-neutral-800 dark:hover:text-neutral-200"
          }`}
        >
          {i.label}
        </Link>
      ))}
    </nav>
  );
}

export function Empty({ children }: { children: React.ReactNode }) {
  return <p className="py-8 text-sm text-neutral-500">{children}</p>;
}

export function ErrorNote({ error }: { error: unknown }) {
  const message = error instanceof Error ? error.message : String(error);
  return (
    <p role="alert" className="py-4 text-sm text-red-600">
      {message}
    </p>
  );
}
