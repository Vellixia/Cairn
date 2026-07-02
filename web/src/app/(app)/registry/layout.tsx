"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { Package, KeyRound, History } from "lucide-react";
import { cn } from "@/lib/utils";

const TABS = [
  { label: "Packs", href: "/registry/packs", icon: Package },
  { label: "Trusted Keys", href: "/registry/trust", icon: KeyRound },
  { label: "Revocations", href: "/registry/revocations", icon: History },
];

export default function RegistryLayout({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();

  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">Pack registry</h1>
        <p className="text-sm text-muted-foreground">
          Published .cairnpkg packs, trusted signing keys, and the revocation log.
        </p>
      </header>

      <div className="hidden border-b border-line md:block">
        <nav className="-mb-px flex flex-wrap gap-1" aria-label="Registry sections">
          {TABS.map((t) => {
            const isActive = pathname === t.href || pathname.startsWith(t.href + "/");
            const Icon = t.icon;
            return (
              <Link
                key={t.href}
                href={t.href}
                aria-current={isActive ? "page" : undefined}
                className={cn(
                  "inline-flex items-center gap-1.5 rounded-t-md border border-b-0 px-3 py-1.5 text-sm font-medium transition-colors",
                  isActive
                    ? "border-line bg-card text-foreground"
                    : "border-transparent text-muted-foreground hover:bg-card/50 hover:text-foreground",
                )}
              >
                <Icon className="h-4 w-4" />
                {t.label}
              </Link>
            );
          })}
        </nav>
      </div>

      <div className="md:hidden">
        <label className="sr-only" htmlFor="registry-section-select">Section</label>
        <div className="relative">
          <select
            id="registry-section-select"
            value={pathname}
            onChange={(e) => { window.location.href = e.target.value; }}
            className="w-full appearance-none rounded-md border border-line bg-card px-3 py-2 pr-8 text-sm font-medium"
          >
            {TABS.map((t) => (
              <option key={t.href} value={t.href}>{t.label}</option>
            ))}
          </select>
          <History className="pointer-events-none absolute right-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        </div>
      </div>

      {children}
    </div>
  );
}
