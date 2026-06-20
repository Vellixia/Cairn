"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
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

type Section = { title: string; items: Item[] };

const SECTIONS: Section[] = [
  {
    title: "Server",
    items: [
      { href: "/dashboard", label: "Overview", icon: LayoutDashboard },
      { href: "/dashboard/settings", label: "Settings", icon: Settings },
    ],
  },
  {
    title: "Memory",
    items: [
      { href: "/dashboard/memory", label: "Memories", icon: Brain },
      { href: "/dashboard/memory/recall", label: "Recall", icon: Search },
      { href: "/dashboard/memory/wakeup", label: "Wakeup", icon: Sparkles },
      { href: "/dashboard/memory/graph", label: "Graph", icon: Network },
    ],
  },
  {
    title: "Context",
    items: [
      { href: "/dashboard/context", label: "Inspector", icon: FileSearch },
      { href: "/dashboard/context/assemble", label: "Assemble", icon: Layers },
      { href: "/dashboard/savings", label: "Savings", icon: PiggyBank },
    ],
  },
  {
    title: "Reliability",
    items: [
      { href: "/dashboard/reliability", label: "Score", icon: Activity },
      { href: "/dashboard/reliability/anchor", label: "Anchor", icon: Target },
      { href: "/dashboard/reliability/checkpoints", label: "Checkpoints", icon: History },
      { href: "/dashboard/reliability/drift", label: "Drift center", icon: ShieldAlert },
    ],
  },
  {
    title: "Share",
    items: [
      { href: "/dashboard/share/sanitize", label: "Sanitize", icon: ShieldCheck },
      { href: "/dashboard/share/export", label: "Bundles", icon: Package },
      { href: "/dashboard/pool", label: "Pool", icon: Users },
    ],
  },
  {
    title: "Personalization",
    items: [
      { href: "/dashboard/profile", label: "Profile", icon: UserCircle },
    ],
  },
  {
    title: "Devices",
    items: [
      { href: "/dashboard/devices", label: "Tokens", icon: KeyRound },
      { href: "/dashboard/devices/pair", label: "Pair new", icon: UserPlus },
      { href: "/dashboard/devices/audit", label: "Audit", icon: FileClock },
    ],
  },
];

export function CairnSidebar() {
  const pathname = usePathname();
  return (
    <Sidebar collapsible="offcanvas" className="border-r border-line">
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
          </SidebarGroup>
        ))}
      </SidebarContent>
    </Sidebar>
  );
}
