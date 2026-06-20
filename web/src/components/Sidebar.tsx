"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
} from "@/components/ui/sidebar";
import Logo from "@/components/Logo";

type Item = { href: string; label: string };

type Section = { title: string; items: Item[] };

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

export function CairnSidebar() {
  const pathname = usePathname();
  return (
    <SidebarProvider>
      <Sidebar collapsible="none" className="border-r border-line">
        <SidebarHeader className="border-b border-line">
          <div className="flex items-center gap-2 px-2 py-2">
            <Logo size={26} />
            <span className="font-semibold tracking-tight">Cairn</span>
          </div>
        </SidebarHeader>
        <SidebarContent>
          {SECTIONS.map((s) => (
            <SidebarGroup key={s.title}>
              <SidebarGroupLabel>{s.title}</SidebarGroupLabel>
              <SidebarGroupContent>
                <SidebarMenu>
                  {s.items.map((it) => {
                    const active =
                      pathname === it.href ||
                      (it.href !== "/dashboard" &&
                        pathname?.startsWith(it.href + "/")) ||
                      (it.href === "/dashboard" && pathname === "/dashboard");
                    return (
                      <SidebarMenuItem key={it.href}>
                        <SidebarMenuButton asChild isActive={active} size="sm">
                          <Link
                            href={it.href}
                            aria-current={active ? "page" : undefined}
                          >
                            {it.label}
                          </Link>
                        </SidebarMenuButton>
                      </SidebarMenuItem>
                    );
                  })}
                </SidebarMenu>
              </SidebarGroupContent>
            </SidebarGroup>
          ))}
        </SidebarContent>
      </Sidebar>
    </SidebarProvider>
  );
}
