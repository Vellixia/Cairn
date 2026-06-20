"use client";

import { useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { useUIStore } from "@/lib/stores/ui";

const SHORTCUTS: { keys: string; desc: string }[] = [
  { keys: "⌘K / Ctrl+K", desc: "Toggle the command palette" },
  { keys: "?", desc: "Toggle this shortcuts modal" },
  { keys: "esc", desc: "Close any open dialog" },
];

export function Shortcuts() {
  const open = useUIStore((s) => s.shortcutsOpen);
  const setOpen = useUIStore((s) => s.setShortcutsOpen);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "?" && !isTyping(e.target)) {
        e.preventDefault();
        setOpen(!useUIStore.getState().shortcutsOpen);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Keyboard shortcuts</DialogTitle>
          <DialogDescription>
            Quick navigation across the Cairn dashboard.
          </DialogDescription>
        </DialogHeader>
        <ul className="mt-2 space-y-2">
          {SHORTCUTS.map((s) => (
            <li
              key={s.keys}
              className="flex items-baseline justify-between gap-3 text-sm"
            >
              <span className="text-muted-foreground">{s.desc}</span>
              <Kbd>{s.keys}</Kbd>
            </li>
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  );
}

function isTyping(target: EventTarget | null): boolean {
  const el = target as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    el.isContentEditable
  );
}
