"use client";

import { Badge } from "@/components/ui/badge";
import { useStreamStatus } from "@/lib/stores/events";

/** Live/Connecting/Polling indicator for the SSE stream - sits next to the health badge. */
export function LiveStatus() {
  const status = useStreamStatus();

  if (status === "live") {
    return (
      <Badge
        variant="secondary"
        className="font-normal"
        title="Live updates connected (SSE)"
      >
        <span className="mr-1.5 h-1.5 w-1.5 rounded-full bg-emerald-500" />
        Live
      </Badge>
    );
  }
  if (status === "connecting") {
    return (
      <Badge
        variant="outline"
        className="font-normal text-muted-foreground"
        title="Connecting to live updates"
      >
        <span className="mr-1.5 h-1.5 w-1.5 rounded-full bg-muted-foreground" />
        Connecting
      </Badge>
    );
  }
  return (
    <Badge
      variant="outline"
      className="font-normal text-muted-foreground"
      title="Live updates unavailable - falling back to polling"
    >
      <span className="mr-1.5 h-1.5 w-1.5 rounded-full bg-amber-500" />
      Polling
    </Badge>
  );
}
