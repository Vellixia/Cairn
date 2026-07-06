"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import {
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/components/ui/command";
import {
  LayoutDashboard,
  Settings,
  Brain,
  Search,
  FileSearch,
  Activity,
  Network,
  KeyRound,
  FileClock,
  UserCircle,
  MessagesSquare,
  PiggyBank,
  Smartphone,
  Bot,
  FolderGit2,
  FileText,
} from "lucide-react";
import { useUIStore } from "@/lib/stores/ui";

interface Item {
  id: string;
  label: string;
  hint?: string;
  shortcut?: string;
  group: "Navigate" | "Memory" | "Devices" | "Personalization";
  icon: React.ComponentType<{ className?: string }>;
  action: () => void;
}

export function CommandPalette() {
  const router = useRouter();
  const open = useUIStore((s) => s.commandOpen);
  const setOpen = useUIStore((s) => s.setCommandOpen);
  const setShortcutsOpen = useUIStore((s) => s.setShortcutsOpen);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen(!useUIStore.getState().commandOpen);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);

  const nav = (href: string) => () => {
    setOpen(false);
    router.push(href);
  };

  const items: Item[] = [
    // --- Navigate ---
    { id: "nav-now", label: "Now", group: "Navigate", icon: LayoutDashboard, action: nav("/") },
    { id: "nav-mem", label: "Memory browser", group: "Navigate", icon: Brain, action: nav("/memory") },
    { id: "nav-graph", label: "Memory . Graph", group: "Navigate", icon: Network, action: nav("/memory/graph") },
    { id: "nav-arch", label: "Memory . Architecture report", group: "Navigate", icon: FileSearch, action: nav("/memory/architecture") },
    { id: "nav-heat", label: "Memory . Activity heatmap", group: "Navigate", icon: Activity, action: nav("/memory/heatmap") },
    { id: "nav-savings", label: "Memory . Savings", group: "Navigate", icon: PiggyBank, action: nav("/memory/savings") },
    { id: "nav-projects", label: "Projects", group: "Navigate", icon: FolderGit2, action: nav("/projects") },
    { id: "nav-documents", label: "Documents", group: "Navigate", icon: FileText, action: nav("/documents") },
    { id: "nav-automation", label: "Automation", group: "Navigate", icon: Bot, action: nav("/automation") },
    { id: "nav-you", label: "You hub", group: "Navigate", icon: UserCircle, action: nav("/you") },
    { id: "nav-profile", label: "You . Profile", group: "Navigate", icon: UserCircle, action: nav("/you?tab=profile") },
    { id: "nav-tokens", label: "You . Device tokens", group: "Navigate", icon: KeyRound, action: nav("/you?tab=tokens") },
    { id: "nav-audit", label: "You . Audit log", group: "Navigate", icon: FileClock, action: nav("/you?tab=audit") },
    { id: "nav-sessions", label: "You . Sessions", group: "Navigate", icon: MessagesSquare, action: nav("/you?tab=sessions") },
    { id: "nav-settings", label: "You . Settings", group: "Navigate", icon: Settings, action: nav("/you?tab=settings") },
    { id: "nav-mobile", label: "Mobile companion (PWA)", group: "Navigate", icon: Smartphone, action: nav("/mobile") },
    // --- Actions ---
    { id: "act-search-mem", label: "Search memories", hint: "jump to browser search", group: "Memory", icon: Search, action: nav("/memory?focus=search") },
    { id: "act-issue", label: "Issue a device token", hint: "jump to Tokens", group: "Devices", icon: KeyRound, action: nav("/you?tab=tokens") },
    { id: "act-prefer", label: "View preferences", hint: "jump to Profile", group: "Personalization", icon: UserCircle, action: nav("/you?tab=profile") },
  ];

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput placeholder="Jump to a section, run an action..." />
      <CommandList>
        <CommandEmpty>No matches. Try a section name like &quot;memory&quot; or &quot;tokens&quot;.</CommandEmpty>
        {(["Navigate", "Memory", "Devices", "Personalization"] as const).map((group) => {
          const filtered = items.filter((i) => i.group === group);
          if (filtered.length === 0) return null;
          return (
            <CommandGroup key={group} heading={group}>
              {filtered.map((it) => {
                const Icon = it.icon;
                return (
                  <CommandItem
                    key={it.id}
                    value={`${it.label} ${it.hint ?? ""}`}
                    onSelect={it.action}
                  >
                    <Icon className="h-4 w-4" />
                    <span className="flex-1 truncate">{it.label}</span>
                    {it.hint && (
                      <span className="text-[11px] text-muted-foreground truncate">
                        {it.hint}
                      </span>
                    )}
                  </CommandItem>
                );
              })}
            </CommandGroup>
          );
        })}
      </CommandList>
      <div className="flex items-center justify-between border-t border-line px-3 py-2 text-[11px] text-muted-foreground">
        <span>
          <CommandShortcut>^v</CommandShortcut> navigate .{" "}
          <CommandShortcut>↵</CommandShortcut> select .{" "}
          <CommandShortcut>esc</CommandShortcut> close
        </span>
        <button
          type="button"
          className="text-[11px] text-muted-foreground hover:text-foreground"
          onClick={() => {
            setOpen(false);
            setShortcutsOpen(true);
          }}
        >
          <CommandShortcut>?</CommandShortcut> shortcuts
        </button>
      </div>
    </CommandDialog>
  );
}
