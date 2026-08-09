"use client";

import { useQuery } from "@tanstack/react-query";
import { ArrowUpRight, CircleCheck, CircleDashed } from "lucide-react";
import { api } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";

/**
 * What this deployment is running, and whether something newer exists.
 *
 * The server does the lookup and caches it, so this costs one cheap request
 * however many people have the page open.
 */
export function useVersion() {
  return useQuery({
    queryKey: ["version"],
    queryFn: () => api.version(),
    // A release does not appear twice an hour, and the server caches anyway.
    staleTime: 30 * 60 * 1000,
    retry: false,
  });
}

/** The line in the sidebar footer. */
export function VersionLine() {
  const version = useVersion();

  if (version.isLoading) return <Skeleton className="h-4 w-24" />;
  if (!version.data) return null;

  const { current, latest, update_available, checked_at } = version.data;

  if (update_available && latest) {
    return (
      <a
        href={latest.url}
        target="_blank"
        rel="noreferrer noopener"
        data-testid="update-available"
        className="hover:bg-sidebar-accent flex items-center gap-1.5 rounded-md px-1 py-0.5 text-xs transition"
      >
        <Badge variant="secondary" className="gap-1">
          <ArrowUpRight className="size-3" />
          {latest.version}
        </Badge>
        <span className="text-muted-foreground">available</span>
      </a>
    );
  }

  const label = checked_at
    ? `Up to date. Last checked ${new Date(checked_at).toLocaleString()}.`
    : "Could not reach GitHub to check for a newer release.";

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <span
            data-testid="version-line"
            className="text-muted-foreground flex items-center gap-1.5 px-1 text-xs"
          />
        }
      >
        {checked_at ? (
          <CircleCheck className="size-3" />
        ) : (
          <CircleDashed className="size-3" />
        )}
        v{current}
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The version on the sign-in page.
 *
 * Someone diagnosing a deployment should not have to sign in to learn which
 * one they are looking at.
 */
export function VersionFooter() {
  const version = useVersion();
  if (!version.data) return null;
  const { current, latest, update_available } = version.data;
  return (
    <p className="text-muted-foreground mt-2 text-center text-xs">
      Cairn v{current}
      {update_available && latest && (
        <>
          {" · "}
          <a
            href={latest.url}
            target="_blank"
            rel="noreferrer noopener"
            className="underline underline-offset-2"
          >
            {latest.version} available
          </a>
        </>
      )}
    </p>
  );
}
