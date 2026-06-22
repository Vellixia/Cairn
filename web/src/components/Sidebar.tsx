"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useState } from "react";
import { ChevronDown } from "lucide-react";
import {
  LayoutDashboard,
  Settings,
  Brain,
  Search,
  Sparkles,
  FileSearch,
  Layers,
  Activity,
  Target,
  History,
  ShieldAlert,
  PiggyBank,
  Network,
  ShieldCheck,
  Package,
  Users,
  KeyRound,
  UserPlus,
  FileClock,
  UserCircle,
  Library,
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
import { cn } from "@/lib/utils";
import Logo from "@/components/Logo";

type Item = { href: string; label: string; icon: LucideIcon };

type Group = { id: string; label: string; items: Item[] };

const GROUPS: Group[] = [
  {
    id: "now",
    label: "Now",
    items: [{ href: "/dashboard", label: "Overview", icon: LayoutDashboard }],
  },
  {
    id: "memory",
    label: "Memory",
    items: [
      { href: "/dashboard/memory", label: "Memories", icon: Brain },
      { href: "/dashboard/memory/recall", label: "Recall", icon: Search },
      { href: "/dashboard/memory/wakeup", label: "Wakeup", icon: Sparkles },
      { href: "/dashboard/memory/graph", label: "Graph", icon: Network },
    ],
  },
  {
    id: "context",
    label: "Context",
    items: [
      { href: "/dashboard/context", label: "Inspector", icon: FileSearch },
      { href: "/dashboard/context/assemble", label: "Assemble", icon: Layers },
      { href: "/dashboard/savings", label: "Savings", icon: PiggyBank },
    ],
  },
  {
    id: "reliability",
    label: "Reliability",
    items: [
      { href: "/dashboard/reliability", label: "Score", icon: Activity },
      { href: "/dashboard/reliability/anchor", label: "Anchor", icon: Target },
      {
        href: "/dashboard/reliability/checkpoints",
        label: "Checkpoints",
        icon: History,
      },
      { href: "/dashboard/reliability/drift", label: "Drift center", icon: ShieldAlert },
    ],
  },
  {
    id: "share",
    label: "Share",
    items: [
      { href: "/dashboard/share/sanitize", label: "Sanitize", icon: ShieldCheck },
      { href: "/dashboard/share/export", label: "Bundles", icon: Package },
      { href: "/dashboard/pool", label: "Pool", icon: Users },
      { href: "/dashboard/registry", label: "Registry", icon: Library },
    ],
  },
  {
    id: "personalization",
    label: "Personalization",
    items: [{ href: "/dashboard/profile", label: "Profile", icon: UserCircle }],
  },
  {
    id: "devices",
    label: "Devices",
    items: [
      { href: "/dashboard/devices", label: "Tokens", icon: KeyRound },
      { href: "/dashboard/devices/pair", label: "Pair new", icon: UserPlus },
      { href: "/dashboard/devices/audit", label: "Audit", icon: FileClock },
    ],
  },
  {
    id: "system",
    label: "System",
    items: [{ href: "/dashboard/settings", label: "Settings", icon: Settings }],
  },
];

const STORAGE_KEY = "cairn-sidebar-v1";

type Persisted = Record<string, boolean>;

const DEFAULT_OPEN: Persisted = {
  now: true,
  memory: true,
  context: false,
  reliability: false,
  share: false,
  personalization: false,
  devices: false,
  system: false,
};

function loadOpen(): Persisted {
  if (typeof window === "undefined") return DEFAULT_OPEN;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_OPEN;
    const parsed = JSON.parse(raw) as Persisted;
    if (!parsed || typeof parsed !== "object") return DEFAULT_OPEN;
    return { ...DEFAULT_OPEN, ...parsed, now: true };
  } catch {
    return DEFAULT_OPEN;
  }
}

function isActive(pathname: string | null, href: string): boolean {
  if (!pathname) return false;
  if (href === "/dashboard") return pathname === "/dashboard";
  return pathname === href || pathname.startsWith(href + "/");
}

function CollapsibleGroup({
  group,
  pathname,
  open,
  onToggle,
}: {
  group: Group;
  pathname: string | null;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <SidebarGroup>
      {group.id === "now" ? (
        <SidebarGroupLabel
          className={cn(
            "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs font-semibold uppercase tracking-wide text-muted-foreground",
            !open && "mb-0",
          )}
        >
          <span>{group.label}</span>
        </SidebarGroupLabel>
      ) : (
      <SidebarGroupLabel asChild>
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={open}
          className={cn(
            "flex w-full items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left text-xs font-semibold uppercase tracking-wide text-muted-foreground transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
            !open && "mb-0",
          )}
        >
          <span>{group.label}</span>
          <ChevronDown
            className={cn(
              "size-3.5 shrink-0 transition-transform duration-200",
              !open && "-rotate-90",
            )}
            aria-hidden="true"
          />
        </button>
      </SidebarGroupLabel>
      )}
      {open ? (
        <SidebarGroupContent>
          <SidebarMenu>
            {group.items.map((it) => {
              const active = isActive(pathname, it.href);
              const Icon = it.icon;
              return (
                <SidebarMenuItem key={it.href}>
                  <SidebarMenuButton asChild isActive={active} size="sm">
                    <Link
                      href={it.href}
                      aria-current={active ? "page" : undefined}
                    >
                      <Icon />
                      <span>{it.label}</span>
                    </Link>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              );
            })}
          </SidebarMenu>
        </SidebarGroupContent>
      ) : null}
    </SidebarGroup>
  );
}

export function CairnSidebar() {
  const pathname = usePathname();
  const [openMap, setOpenMap] = useState<Persisted>(DEFAULT_OPEN);
  const [hydrated, setHydrated] = useState(false);

  useEffect(() => {
    setOpenMap(loadOpen());
    setHydrated(true);
  }, []);

  useEffect(() => {
    if (!hydrated) return;
    try {
      window.localStorage.setItem(STORAGE_KEY, JSON.stringify(openMap));
    } catch {
      /* ignore quota / private mode */
    }
  }, [openMap, hydrated]);

  const toggle = (id: string) =>
    setOpenMap((m) => (id === "now" ? m : { ...m, [id]: !(m[id] ?? false) }));

  return (
    <Sidebar collapsible="offcanvas" className="border-r border-line">
      <SidebarHeader className="border-b border-line">
        <div className="flex items-center gap-2 px-2 py-2">
          <Logo size={26} />
          <span className="font-semibold tracking-tight">Cairn</span>
        </div>
      </SidebarHeader>
      <SidebarContent>
        {GROUPS.map((g) => (
          <CollapsibleGroup
            key={g.id}
            group={g}
            pathname={pathname}
            open={openMap[g.id] ?? false}
            onToggle={() => toggle(g.id)}
          />
        ))}
      </SidebarContent>
    </Sidebar>
  );
}
