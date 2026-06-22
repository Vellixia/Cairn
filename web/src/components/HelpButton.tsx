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

/**
 * Compact "?" button that opens a dialog with what/how/impact help text.
 * One per page — never inline. Replaces the old InfoCard pattern.
 */
export function HelpButton({ content }: { content: HelpContent }) {
  return (
    <Dialog>
      <DialogTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 rounded-full text-muted-foreground hover:text-foreground"
          aria-label={`Help: ${content.title}`}
        >
          <QuestionMarkIcon className="h-4 w-4" />
        </Button>
      </DialogTrigger>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{content.title}</DialogTitle>
          <DialogDescription>
            <span className="block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground mt-2 mb-1">
              What this is
            </span>
            <span className="block text-sm text-foreground">{content.what}</span>
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3 text-sm">
          <div>
            <span className="block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground mb-1">
              How to use it
            </span>
            <ul className="space-y-1.5 text-foreground">
              {content.how.map((line, i) => (
                <li key={i} className="flex gap-2">
                  <span className="text-muted-foreground select-none">·</span>
                  <span>{line}</span>
                </li>
              ))}
            </ul>
          </div>
          <div>
            <span className="block text-[11px] font-semibold uppercase tracking-wider text-muted-foreground mb-1">
              Impact on Cairn
            </span>
            <span className="block text-foreground">{content.impact}</span>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}