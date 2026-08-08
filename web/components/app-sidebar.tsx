"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTheme } from "next-themes";
import {
  Brain,
  ChevronsUpDown,
  FolderGit2,
  KeyRound,
  LayoutDashboard,
  ListChecks,
  LogOut,
  Monitor,
  Moon,
  Mountain,
  RefreshCw,
  Sun,
  Terminal,
} from "lucide-react";
import { api } from "@/lib/api";
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

  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => api.projects(),
  });

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
        { href: `/projects/${activeId}/memory`, label: "Memory", icon: Brain },
        { href: `/projects/${activeId}/sync`, label: "Sync", icon: RefreshCw },
      ]
    : [];

  return (
    <Sidebar collapsible="icon">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton size="lg" render={<Link href="/" />}>
              <div className="bg-sidebar-primary text-sidebar-primary-foreground flex aspect-square size-8 items-center justify-center rounded-lg">
                <Mountain className="size-4" />
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
            <DropdownMenuLabel className="text-muted-foreground text-xs">
              Theme
            </DropdownMenuLabel>
            <DropdownMenuGroup>
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
