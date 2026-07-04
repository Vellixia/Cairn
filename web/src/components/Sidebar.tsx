"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import {
  LayoutDashboard,
  Brain,
  FolderGit2,
  FileText,
  Bot,
  UserCircle,
  type LucideIcon,
} from "lucide-react";
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
} from "@/components/ui/sidebar";
import Logo from "@/components/Logo";

type Item = { href: string; label: string; icon: LucideIcon };

// Observe: pure observability - what exists, what happened, what the machine decided.
// Admin: security/account surfaces (tokens, pairing, audit, settings) - the only place
// manual input stays. Trust folded into Automation; Registry is agent-facing (CLI/MCP only).
const MONITOR_ITEMS: Item[] = [
  { href: "/", label: "Now", icon: LayoutDashboard },
  { href: "/memory", label: "Memory", icon: Brain },
  { href: "/projects", label: "Projects", icon: FolderGit2 },
  { href: "/documents", label: "Documents", icon: FileText },
  { href: "/automation", label: "Automation", icon: Bot },
];

const ADMIN_ITEMS: Item[] = [{ href: "/you", label: "You", icon: UserCircle }];

const STORAGE_KEY = "cairn-sidebar-v4";

function isActive(pathname: string | null, href: string): boolean {
  if (!pathname) return false;
  const [path] = href.split("?");
  if (path === "/") {
    return pathname === "/" || pathname === "";
  }
  return pathname === path || pathname.startsWith(path + "/");
}

function NavLink({
  item,
  active,
}: {
  item: Item;
  active: boolean;
}) {
  const Icon = item.icon;
  return (
    <SidebarMenuButton asChild isActive={active} size="sm">
      <Link
        href={item.href}
        aria-current={active ? "page" : undefined}
        onClick={() => {
          if (typeof window === "undefined") return;
          try {
            window.localStorage.setItem(STORAGE_KEY, "1");
          } catch {
            /* ignore */
          }
        }}
      >
        <Icon />
        <span>{item.label}</span>
      </Link>
    </SidebarMenuButton>
  );
}

export function CairnSidebar() {
  const pathname = usePathname();
  const [hydrated, setHydrated] = useState(false);

  useEffect(() => {
    // Migration: drop any prior sidebar keys; v4 = grouped (Monitor/Admin).
    for (const k of ["cairn-sidebar-v1", "cairn-sidebar-v2", "cairn-sidebar-v3", "cairn-infocard-dismissed-v1"]) {
      try {
        window.localStorage.removeItem(k);
      } catch {
        /* ignore */
      }
    }
    try {
      window.sessionStorage.removeItem("cairn-infocard-dismissed-v1");
    } catch {
      /* ignore */
    }
    try {
      window.localStorage.setItem(STORAGE_KEY, "1");
    } catch {
      /* ignore */
    }
    setHydrated(true);
  }, []);

  return (
    <Sidebar collapsible="offcanvas" className="border-r border-line">
      <SidebarHeader className="border-b border-line">
        <div className="flex items-center gap-2 px-2 py-2">
          <Logo size={26} />
          <span className="font-semibold tracking-tight">Cairn</span>
        </div>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel className="px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Monitor
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {hydrated
                ? MONITOR_ITEMS.map((it) => (
                    <SidebarMenuItem key={it.href}>
                      <NavLink item={it} active={isActive(pathname, it.href)} />
                    </SidebarMenuItem>
                  ))
                : null}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
        <SidebarGroup>
          <SidebarGroupLabel className="px-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            Admin
          </SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {hydrated
                ? ADMIN_ITEMS.map((it) => (
                    <SidebarMenuItem key={it.href}>
                      <NavLink item={it} active={isActive(pathname, it.href)} />
                    </SidebarMenuItem>
                  ))
                : null}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
    </Sidebar>
  );
}