"use client";

import { useEventStream } from "@/lib/events";

/** Invisible global mount point for the SSE connection - see `useEventStream`. */
export function EventStreamProvider() {
  useEventStream();
  return null;
}
