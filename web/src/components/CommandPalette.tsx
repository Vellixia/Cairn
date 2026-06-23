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
  Sparkles,
  FileSearch,
  Layers,
  Activity,
  Target,
  History,
  Network,
  ShieldCheck,
  Package,
  Users,
  KeyRound,
  UserPlus,
  FileClock,
  Library,
  UserCircle,
  ShieldAlert,
  MessagesSquare,
  PiggyBank,
} from "lucide-react";
import { useUIStore } from "@/lib/stores/ui";

interface Item {
  id: string;
  label: string;
  hint?: string;
  shortcut?: string;
  group: "Navigate" | "Memory" | "Reliability" | "Devices" | "Share" | "Personalization" | "Sessions";
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
    { id: "nav-overview", label: "Overview", group: "Navigate", icon: LayoutDashboard, action: nav("/dashboard") },
    { id: "nav-settings", label: "Settings", group: "Navigate", icon: Settings, action: nav("/dashboard/settings") },
    { id: "nav-mem", label: "Memories · Remember", group: "Navigate", icon: Brain, action: nav("/dashboard/memory") },
    { id: "nav-recall", label: "Memories · Recall", group: "Navigate", icon: Search, action: nav("/dashboard/memory/recall") },
    { id: "nav-wakeup", label: "Memories · Wakeup", group: "Navigate", icon: Sparkles, action: nav("/dashboard/memory/wakeup") },
    { id: "nav-graph", label: "Memories · Graph", group: "Navigate", icon: Network, action: nav("/dashboard/memory/graph") },
    { id: "nav-ctx", label: "Context · Inspector", group: "Navigate", icon: FileSearch, action: nav("/dashboard/context") },
    { id: "nav-asm", label: "Context · Assemble", group: "Navigate", icon: Layers, action: nav("/dashboard/context/assemble") },
    { id: "nav-savings", label: "Savings & recover", group: "Navigate", icon: PiggyBank, action: nav("/dashboard/savings") },
    { id: "nav-rel", label: "Reliability · Score", group: "Navigate", icon: Activity, action: nav("/dashboard/reliability") },
    { id: "nav-anchor", label: "Reliability · Anchor", group: "Navigate", icon: Target, action: nav("/dashboard/reliability/anchor") },
    { id: "nav-cp", label: "Reliability · Checkpoints", group: "Navigate", icon: History, action: nav("/dashboard/reliability/checkpoints") },
    { id: "nav-drift", label: "Reliability · Drift center", group: "Navigate", icon: ShieldAlert, action: nav("/dashboard/reliability/drift") },
    { id: "nav-sessions", label: "Sessions", group: "Navigate", icon: MessagesSquare, action: nav("/dashboard/sessions") },
    { id: "nav-san", label: "Share · Sanitize", group: "Navigate", icon: ShieldCheck, action: nav("/dashboard/share/sanitize") },
    { id: "nav-bun", label: "Share · Bundles", group: "Navigate", icon: Package, action: nav("/dashboard/share/export") },
    { id: "nav-pool", label: "Pool", group: "Navigate", icon: Users, action: nav("/dashboard/pool") },
    { id: "nav-registry", label: "Pack registry", group: "Navigate", icon: Library, action: nav("/dashboard/registry") },
    { id: "nav-devs", label: "Devices · Tokens", group: "Navigate", icon: KeyRound, action: nav("/dashboard/devices") },
    { id: "nav-pair", label: "Devices · Pair new", group: "Navigate", icon: UserPlus, action: nav("/dashboard/devices/pair") },
    { id: "nav-audit", label: "Devices · Audit", group: "Navigate", icon: FileClock, action: nav("/dashboard/devices/audit") },
    { id: "act-remember", label: "Remember something", hint: "jump to Memories", group: "Memory", icon: Brain, action: nav("/dashboard/memory") },
    { id: "act-recall", label: "Recall a memory", hint: "jump to Recall", group: "Memory", icon: Search, action: nav("/dashboard/memory/recall") },
    { id: "act-cp", label: "Create a checkpoint", hint: "jump to Checkpoints", group: "Reliability", icon: History, action: nav("/dashboard/reliability/checkpoints") },
    { id: "act-issue", label: "Issue a device token", hint: "jump to Tokens", group: "Devices", icon: KeyRound, action: nav("/dashboard/devices") },
    { id: "act-san", label: "Sanitize text", hint: "jump to Sanitize", group: "Share", icon: ShieldCheck, action: nav("/dashboard/share/sanitize") },
    { id: "nav-profile", label: "Profile", group: "Personalization", icon: UserCircle, action: nav("/dashboard/profile") },
    { id: "act-prefer", label: "Add a preference", hint: "jump to Profile", group: "Personalization", icon: UserCircle, action: nav("/dashboard/profile") },
  ];

  return (
    <CommandDialog open={open} onOpenChange={setOpen}>
      <CommandInput placeholder="Jump to a section, run an action…" />
      <CommandList>
        <CommandEmpty>No matches. Try a section name like "memory" or "tokens".</CommandEmpty>
        {(["Navigate", "Memory", "Reliability", "Devices", "Share", "Personalization", "Sessions"] as const).map((group) => {
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
          <CommandShortcut>↑↓</CommandShortcut> navigate ·{" "}
          <CommandShortcut>↵</CommandShortcut> select ·{" "}
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
