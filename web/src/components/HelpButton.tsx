"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { QuestionMarkIcon } from "@radix-ui/react-icons";

export interface HelpContent {
  title: string;
  what: string;
  how: string[];
  impact: string;
}

const FALLBACK_HELP: HelpContent = {
  title: "Help",
  what: "This page is part of the Cairn dashboard.",
  how: ["Refer to docs/TESTING.md and docs/ARCHITECTURE.md for the full surface."],
  impact: "Adding a route-specific entry in web/src/components/helpCopy.ts gives this button a real tooltip.",
};

/**
 * Compact "?" button that opens a dialog with what/how/impact help text.
 * One per page --- never inline. Replaces the old InfoCard pattern.
 *
 * If `content` is omitted (e.g. a page that doesn't have a helpCopy entry yet),
 * the button still renders with a generic fallback. This keeps the page from
 * crashing with `TypeError: Cannot read properties of undefined (reading 'title')`
 * and surfaces the missing entry in the help dialog itself.
 */
export function HelpButton({ content }: { content?: HelpContent }) {
  const c: HelpContent = content ?? FALLBACK_HELP;
  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 rounded-full text-muted-foreground hover:text-foreground"
          aria-label={`Help: ${c.title}`}
        >
          <QuestionMarkIcon className="h-4 w-4" />
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{c.title}</DialogTitle>
          <DialogDescription>
            <span className="block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground mt-2 mb-1">
              What this is
            </span>
            <span className="block text-sm text-foreground">{c.what}</span>
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3 text-sm">
          <div>
            <span className="block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground mb-1">
              How to use it
            </span>
            <ul className="space-y-1.5 text-foreground">
              {c.how.map((line, i) => (
                <li key={i} className="flex gap-2">
                  <span className="text-muted-foreground select-none">.</span>
                  <span>{line}</span>
                </li>
              ))}
            </ul>
          </div>
          <div>
            <span className="block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground mb-1">
              Impact on Cairn
            </span>
            <span className="block text-foreground">{c.impact}</span>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}