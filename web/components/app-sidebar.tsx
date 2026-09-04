"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTheme } from "next-themes";
import { toast } from "sonner";
import {
  Activity,
  Boxes,
  Brain,
  ChevronsUpDown,
  FolderGit2,
  HeartPulse,
  KeyRound,
  LayoutDashboard,
  ListChecks,
  LogOut,
  Monitor,
  Moon,
  Plug,
  RefreshCw,
  Search,
  Sun,
  Terminal,
  UserCog,
  Users,
} from "lucide-react";
import { api } from "@/lib/api";
import { CairnMark } from "@/components/logo";
import { VersionLine } from "@/components/version";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
  SidebarRail,
  useSidebar,
} from "@/components/ui/sidebar";

/**
 * The project whose section is open, taken from the URL.
 *
 * The sidebar keeps no state for this: deep-linking into a project has to
 * light up the same navigation as clicking into it.
 */
function useActiveProjectId(pathname: string): string | null {
  const match = pathname.match(/^\/projects\/([^/]+)/);
  return match ? match[1] : null;
}

export function AppSidebar() {
  const pathname = usePathname();
  const activeId = useActiveProjectId(pathname);
  const { isMobile, setOpenMobile } = useSidebar();

  // On a phone the sidebar is a sheet over the page. Navigating without
  // dismissing it leaves the reader looking at the menu they just used, with
  // the page they asked for hidden behind it.
  const closeOnMobile = () => {
    if (isMobile) setOpenMobile(false);
  };

  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => api.projects(),
  });

  // Same query key the user menu below uses, so the role costs no extra
  // request — and comes from the server on this page load rather than from
  // anything cached at sign-in.
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.me() });

  const active = projects.data?.projects.find((p) => p.id === activeId);

  const projectNav = activeId
    ? [
        {
          href: `/projects/${activeId}`,
          label: "Overview",
          icon: LayoutDashboard,
          exact: true,
        },
        { href: `/projects/${activeId}/tasks`, label: "Tasks", icon: ListChecks },
        {
          href: `/projects/${activeId}/sessions`,
          label: "Sessions",
          icon: Terminal,
        },
        {
          href: `/projects/${activeId}/activity`,
          label: "Activity",
          icon: Activity,
        },
        { href: `/projects/${activeId}/memory`, label: "Memory", icon: Brain },
        {
          href: `/projects/${activeId}/retrievals`,
          label: "Retrievals",
          icon: Search,
        },
        { href: `/projects/${activeId}/agents`, label: "Agents", icon: Plug },
        { href: `/projects/${activeId}/domains`, label: "Domains", icon: Boxes },
        { href: `/projects/${activeId}/sync`, label: "Sync", icon: RefreshCw },
      ]
    : [];

  /**
   * The deployment-wide entries, and who is offered them.
   *
   * **`adminOnly` hides a door that will not open; it does not lock one.** Every
   * route behind these entries is gated by the server — `AdminUser` on system
   * health and on the account routes — and a member who types the URL is
   * refused there and shown that refusal. Hiding the entry saves them the trip;
   * it is not what stops them (FR-892).
   *
   * Team curation is *not* admin-only, because reading it is not: the list is
   * visible to every authenticated account, with proposals filtered per caller
   * by the server. Only the ratify and retire actions on that page are
   * administrator work.
   */
  const isAdmin = me.data?.role === "admin";
  const systemNav = [
    { href: "/team", label: "Team", icon: Users, testId: "nav-team" },
    {
      href: "/system",
      label: "System health",
      icon: HeartPulse,
      testId: "nav-system",
      adminOnly: true,
    },
    {
      href: "/admin/users",
      label: "Accounts",
      icon: UserCog,
      testId: "nav-admin-users",
      adminOnly: true,
    },
  ].filter((item) => isAdmin || !item.adminOnly);

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="lg"
              onClick={closeOnMobile}
              render={<Link href="/" />}
            >
              <div className="bg-sidebar-primary text-sidebar-primary-foreground flex aspect-square size-8 items-center justify-center rounded-lg">
                <CairnMark className="size-4" />
              </div>
              <div className="grid flex-1 text-left leading-tight">
                <span className="truncate font-semibold">Cairn</span>
                <span className="text-muted-foreground truncate text-xs">
                  Shared project memory
                </span>
              </div>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>

      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupLabel>Workspace</SidebarGroupLabel>
          <SidebarMenu>
            <SidebarMenuItem>
              <SidebarMenuButton
                isActive={pathname === "/"}
                tooltip="Projects"
                onClick={closeOnMobile}
                render={<Link href="/" data-testid="nav-projects" />}
              >
                <FolderGit2 />
                <span>Projects</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
            <SidebarMenuItem>
              <SidebarMenuButton
                isActive={pathname === "/tokens"}
                tooltip="API tokens"
                onClick={closeOnMobile}
                render={<Link href="/tokens" data-testid="nav-tokens" />}
              >
                <KeyRound />
                <span>API tokens</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarGroup>

        {activeId && (
          <SidebarGroup>
            <SidebarGroupLabel className="truncate">
              {active?.name ?? "Project"}
            </SidebarGroupLabel>
            <SidebarMenu>
              {projectNav.map((item) => {
                const isActive = item.exact
                  ? pathname === item.href
                  : pathname.startsWith(item.href);
                return (
                  <SidebarMenuItem key={item.href}>
                    <SidebarMenuButton
                      isActive={isActive}
                      tooltip={item.label}
                      onClick={closeOnMobile}
                      render={
                        <Link
                          href={item.href}
                          data-testid={`nav-${item.label.toLowerCase()}`}
                        />
                      }
                    >
                      <item.icon />
                      <span>{item.label}</span>
                    </SidebarMenuButton>
                  </SidebarMenuItem>
                );
              })}
            </SidebarMenu>
          </SidebarGroup>
        )}

        <SidebarGroup>
          <SidebarGroupLabel>Server</SidebarGroupLabel>
          <SidebarMenu>
            {systemNav.map((item) => (
              <SidebarMenuItem key={item.href}>
                <SidebarMenuButton
                  isActive={pathname.startsWith(item.href)}
                  tooltip={item.label}
                  onClick={closeOnMobile}
                  render={<Link href={item.href} data-testid={item.testId} />}
                >
                  <item.icon />
                  <span>{item.label}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            ))}
          </SidebarMenu>
        </SidebarGroup>

        <SidebarGroup>
          <SidebarGroupLabel>Your projects</SidebarGroupLabel>
          <SidebarMenu>
            {projects.isLoading &&
              [0, 1, 2].map((i) => (
                <SidebarMenuItem key={i}>
                  <SidebarMenuSkeleton showIcon />
                </SidebarMenuItem>
              ))}
            {projects.data?.projects.map((p) => (
              <SidebarMenuItem key={p.id}>
                <SidebarMenuButton
                  isActive={p.id === activeId}
                  tooltip={p.name}
                  onClick={closeOnMobile}
                  render={<Link href={`/projects/${p.id}`} />}
                >
                  <FolderGit2 />
                  <span className="truncate">{p.name}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            ))}
          </SidebarMenu>
        </SidebarGroup>
      </SidebarContent>

      <SidebarFooter>
        {/* Hidden when collapsed to icons: a version string in a 3rem rail is
            noise, and the tooltip cannot be reached from a collapsed item. */}
        <div className="group-data-[collapsible=icon]:hidden">
          <VersionLine />
        </div>
        <UserMenu />
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  );
}

function UserMenu() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const { theme, setTheme } = useTheme();

  const me = useQuery({ queryKey: ["me"], queryFn: () => api.me() });

  const signOut = useMutation({
    mutationFn: () => api.logout(),
    onSuccess: () => {
      // Drop every cached answer: the next user must not see this one's data.
      queryClient.clear();
      router.push("/login");
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const initials = (me.data?.display_name ?? "?")
    .split(" ")
    .map((w) => w[0])
    .slice(0, 2)
    .join("")
    .toUpperCase();

  return (
    <SidebarMenu>
      <SidebarMenuItem>
        <DropdownMenu>
          <DropdownMenuTrigger
            render={<SidebarMenuButton size="lg" data-testid="user-menu" />}
          >
            <div className="bg-muted text-muted-foreground flex aspect-square size-8 items-center justify-center rounded-lg text-xs font-medium">
              {initials}
            </div>
            <div className="grid flex-1 text-left leading-tight">
              <span className="truncate text-sm font-medium">
                {me.data?.display_name ?? "Signed out"}
              </span>
              <span className="text-muted-foreground truncate text-xs">
                {me.data?.email ?? "Not signed in"}
              </span>
            </div>
            <ChevronsUpDown className="ml-auto size-4" />
          </DropdownMenuTrigger>
          <DropdownMenuContent side="top" align="start" className="min-w-56">
            <DropdownMenuGroup>
              <DropdownMenuLabel className="text-muted-foreground text-xs">
                Theme
              </DropdownMenuLabel>
              {(
                [
                  ["light", "Light", Sun],
                  ["dark", "Dark", Moon],
                  ["system", "System", Monitor],
                ] as const
              ).map(([value, label, Icon]) => (
                <DropdownMenuItem
                  key={value}
                  onClick={() => setTheme(value)}
                  className={theme === value ? "bg-accent" : undefined}
                >
                  <Icon className="size-4" />
                  {label}
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem render={<Link href="/tokens" />}>
              <KeyRound className="size-4" />
              API tokens
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onClick={() => signOut.mutate()}
              data-testid="sign-out"
            >
              <LogOut className="size-4" />
              Sign out
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </SidebarMenuItem>
    </SidebarMenu>
  );
}
