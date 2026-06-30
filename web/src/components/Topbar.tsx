"use client";

import { useRouter } from "next/navigation";
import { Smartphone } from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Badge } from "@/components/ui/badge";
import { Kbd } from "@/components/ui/kbd";
import { Button } from "@/components/ui/button";
import { SidebarTrigger } from "@/components/ui/sidebar";
import { useHealthQuery, useLogoutMutation } from "@/lib/queries";
import { useMeStore } from "@/lib/stores/me";
import { useUIStore } from "@/lib/stores/ui";

export function Topbar() {
  const router = useRouter();
  const me = useMeStore((s) => s.me);
  const toggleCommand = useUIStore((s) => s.toggleCommand);
  const logout = useLogoutMutation();
  const health = useHealthQuery();

  const healthy = health.data?.status === "ok";
  const initial = (me?.username ?? "?").slice(0, 1).toUpperCase();

  function handleLogout() {
    logout.mutate(undefined, {
      onSettled: () => router.replace("/login"),
    });
  }

  return (
    <header className="sticky top-0 z-10 border-b border-line bg-background/80 backdrop-blur supports-[backdrop-filter]:bg-background/60">
      <div className="flex items-center justify-between gap-4 px-5 py-3">
        <div className="flex items-center gap-3 text-sm text-muted-foreground">
          <SidebarTrigger className="md:hidden" />
          <Button
            variant="outline"
            size="sm"
            className="h-7 px-2 font-mono text-[11px]"
            onClick={toggleCommand}
            aria-label="Open command palette"
          >
            <Kbd>⌘K</Kbd>
            <span className="ml-1.5 hidden sm:inline">jump to anything</span>
          </Button>
          <Button
            variant="outline"
            size="sm"
            className="h-7 px-2"
            onClick={() => router.push("/mobile")}
            aria-label="Open mobile companion"
            title="Mobile companion"
          >
            <Smartphone className="h-3.5 w-3.5" />
          </Button>
        </div>
        <div className="flex items-center gap-4">
          {healthy ? (
            <Badge variant="secondary" className="font-normal">
              <span className="mr-1.5 h-1.5 w-1.5 rounded-full bg-emerald-500" />
              healthy
            </Badge>
          ) : (
            <Badge variant="destructive" className="font-normal">
              <span className="mr-1.5 h-1.5 w-1.5 rounded-full bg-red-500" />
              {health.isError ? "offline" : "..."}
            </Badge>
          )}
          {me && (
            <span className="hidden text-xs text-muted-foreground sm:inline">
              signed in as{" "}
              <span className="font-medium text-foreground">{me.username}</span>
            </span>
          )}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="secondary"
                size="icon"
                className="h-7 w-7 rounded-full text-xs"
                aria-label="Open profile menu"
              >
                {initial}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-44">
              <DropdownMenuLabel>
                {me ? `Signed in as ${me.username}` : "Account"}
              </DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => router.replace("/you?tab=settings")}>
                Settings
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => router.replace("/you?tab=audit")}>
                Audit log
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                className="text-destructive focus:text-destructive"
                onSelect={handleLogout}
              >
                Sign out
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </header>
  );
}
