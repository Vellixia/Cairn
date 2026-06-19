"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import Logo from "@/components/Logo";

type Section = {
  title: string;
  items: { href: string; label: string }[];
};

const SECTIONS: Section[] = [
  {
    title: "Server",
    items: [
      { href: "/dashboard", label: "Overview" },
      { href: "/dashboard/settings", label: "Settings" },
    ],
  },
  {
    title: "Memory",
    items: [
      { href: "/dashboard/memory", label: "Memories" },
      { href: "/dashboard/memory/recall", label: "Recall" },
      { href: "/dashboard/memory/wakeup", label: "Wakeup" },
    ],
  },
  {
    title: "Context",
    items: [
      { href: "/dashboard/context", label: "Inspector" },
      { href: "/dashboard/context/assemble", label: "Assemble" },
    ],
  },
  {
    title: "Reliability",
    items: [
      { href: "/dashboard/reliability", label: "Score" },
      { href: "/dashboard/reliability/anchor", label: "Anchor" },
      { href: "/dashboard/reliability/checkpoints", label: "Checkpoints" },
    ],
  },
  {
    title: "Share",
    items: [
      { href: "/dashboard/share/sanitize", label: "Sanitize" },
      { href: "/dashboard/share/export", label: "Bundles" },
      { href: "/dashboard/pool", label: "Pool" },
    ],
  },
  {
    title: "Devices",
    items: [
      { href: "/dashboard/devices", label: "Tokens" },
      { href: "/dashboard/devices/pair", label: "Pair new" },
      { href: "/dashboard/devices/audit", label: "Audit" },
    ],
  },
];

export function Sidebar() {
  const pathname = usePathname();
  return (
    <aside className="hidden md:flex md:w-60 md:shrink-0 md:flex-col border-r border-line bg-surface/60 backdrop-blur-sm">
      <div className="flex items-center gap-2 px-5 py-4 border-b border-line">
        <Logo size={26} />
        <span className="font-semibold tracking-tight">Cairn</span>
      </div>
      <nav className="flex-1 overflow-y-auto px-3 py-4 space-y-5" aria-label="Main">
        {SECTIONS.map((s) => (
          <div key={s.title}>
            <p className="px-2 mb-1.5 text-[10.5px] uppercase tracking-[0.12em] text-slate">
              {s.title}
            </p>
            <ul className="space-y-0.5">
              {s.items.map((it) => {
                const active =
                  pathname === it.href ||
                  (it.href !== "/dashboard" && pathname?.startsWith(it.href + "/")) ||
                  (it.href === "/dashboard" && pathname === "/dashboard");
                return (
                  <li key={it.href}>
                    <Link
                      href={it.href}
                      aria-current={active ? "page" : undefined}
                      className={`block rounded-md px-2.5 py-1.5 text-sm transition-colors ${
                        active
                          ? "bg-surface2 text-offwhite"
                          : "text-[#b9c2cf] hover:bg-surface2 hover:text-offwhite"
                      }`}
                    >
                      {it.label}
                    </Link>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </nav>
    </aside>
  );
}
