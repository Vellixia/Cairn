"use client";

import Link from "next/link";
import { usePathname, useSearchParams } from "next/navigation";
import { useRouter } from "next/navigation";
import { useEffect, useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { ChevronDown } from "lucide-react";

export type HubTab = {
  id: string;
  label: string;
  content: ReactNode;
};

export function HubTabs({
  view,
  title,
  description,
  tabs,
  defaultTab,
}: {
  // Web redesign v2: the monitor pages dropped the tab-hub pattern; /you is the only hub left.
  view: "you";
  title: string;
  description: string;
  tabs: HubTab[];
  defaultTab: string;
}) {
  const router = useRouter();
  const params = useSearchParams();
  const tabParam = params.get("tab") ?? defaultTab;
  const [hydrated, setHydrated] = useState(false);
  useEffect(() => setHydrated(true), []);
  const active = tabs.find((t) => t.id === tabParam) ?? tabs[0];
  const setTab = (id: string) => {
    const sp = new URLSearchParams(params.toString());
    sp.set("tab", id);
    sp.delete("view");
    router.replace(`/${view}?${sp.toString()}`, { scroll: false });
  };

  return (
    <div className="space-y-6">
      <header className="space-y-1">
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        <p className="text-sm text-muted-foreground">{description}</p>
      </header>

      <div className="md:hidden">
        <label className="sr-only" htmlFor={`hub-${view}-select`}>
          Section
        </label>
        <div className="relative">
          <select
            id={`hub-${view}-select`}
            value={active?.id ?? defaultTab}
            onChange={(e) => setTab(e.target.value)}
            className="w-full appearance-none rounded-md border border-line bg-card px-3 py-2 pr-8 text-sm font-medium"
          >
            {tabs.map((t) => (
              <option key={t.id} value={t.id}>
                {t.label}
              </option>
            ))}
          </select>
          <ChevronDown className="pointer-events-none absolute right-2 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        </div>
      </div>

      <div className="hidden border-b border-line md:block">
        <nav className="-mb-px flex flex-wrap gap-1" aria-label={`${title} sections`}>
          {tabs.map((t) => {
            const isActive = t.id === active?.id;
            return (
              <Link
                key={t.id}
                href={`/${view}?tab=${t.id}`}
                scroll={false}
                onClick={(e) => {
                  e.preventDefault();
                  setTab(t.id);
                }}
                aria-current={isActive ? "page" : undefined}
                className={cn(
                  "rounded-t-md border border-b-0 px-3 py-1.5 text-sm font-medium transition-colors",
                  isActive
                    ? "border-line bg-card text-foreground"
                    : "border-transparent text-muted-foreground hover:bg-card/50 hover:text-foreground",
                )}
              >
                {t.label}
              </Link>
            );
          })}
        </nav>
      </div>

      {hydrated ? <div key={active?.id}>{active?.content}</div> : null}
    </div>
  );
}