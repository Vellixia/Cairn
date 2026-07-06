"use client";

import { useEffect } from "react";
import { useQueryClient, type QueryKey } from "@tanstack/react-query";
import { API_BASE } from "@/lib/api";
import { qk } from "@/lib/queries";
import { useEventStreamStore } from "@/lib/stores/events";

const DEBOUNCE_MS = 300;
const RECONNECT_MS = 3000;

// SSE kind -> query-key prefixes to invalidate (react-query matches by prefix, so ["memory"]
// also invalidates ["memory", "wakeup", 5], ["memory", "list", {...}], ["memory", "detail", id],
// etc - the Memory Browser's keys get live invalidation for free by starting with "memory").
const INVALIDATION_MAP: Record<string, readonly QueryKey[]> = {
  memory: [["memory"], qk.stats, ["projects"]],
  drift: [qk.drift, qk.stats, ["automation"]],
  audit: [qk.devicesAudit, qk.activityAudit, ["automation"]],
  project: [["projects"]],
  document: [["documents"]],
  cron: [["cron"], ["automation"]],
};

/**
 * Subscribes to `GET /api/events` (SSE) for the lifetime of the mounted component and keeps
 * `useEventStreamStore`'s status in sync (`connecting` -> `live` on open, `offline` on error).
 * Invalidations are batched with a 300ms trailing debounce so a burst of events (e.g. a cron
 * job touching many memories) triggers one refetch per affected query rather than one per event.
 *
 * Reconnection: the browser's own `EventSource` auto-retries on a plain network drop, but
 * treats a response whose `Content-Type` isn't `text/event-stream` (e.g. a 503 from a server
 * mid-restart, or a proxy error page) as a *fatal* error - `readyState` goes to `CLOSED` and it
 * never retries again on its own. Verified live: a `docker stop` on the backend left the tab
 * stuck on "Polling" forever even after the server came back, because the browser had already
 * given up. We detect that case (`readyState === CLOSED` after `onerror`) and reconnect
 * ourselves on a fixed delay so a server restart or deploy doesn't require a manual page reload.
 *
 * Mounted once at the dashboard-layout level (inside `SessionGate`, so it only connects once
 * auth is confirmed) - not per-page, so navigating the dashboard doesn't thrash the connection.
 */
export function useEventStream() {
  const qc = useQueryClient();
  const setStatus = useEventStreamStore((s) => s.setStatus);

  useEffect(() => {
    const pending = new Map<string, QueryKey>();
    let debounceTimer: ReturnType<typeof setTimeout> | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let stopped = false;
    let es: EventSource | null = null;

    function flush() {
      for (const key of pending.values()) {
        qc.invalidateQueries({ queryKey: key });
      }
      pending.clear();
      debounceTimer = null;
    }

    function schedule(keys: readonly QueryKey[]) {
      for (const key of keys) pending.set(JSON.stringify(key), key);
      if (debounceTimer) return;
      debounceTimer = setTimeout(flush, DEBOUNCE_MS);
    }

    function connect() {
      if (stopped) return;
      setStatus("connecting");
      const source = new EventSource(`${API_BASE}/api/events`, { withCredentials: true });
      es = source;

      source.onopen = () => setStatus("live");
      source.onerror = () => {
        setStatus("offline");
        if (source.readyState === EventSource.CLOSED && !stopped) {
          source.close();
          reconnectTimer = setTimeout(connect, RECONNECT_MS);
        }
      };

      for (const [kind, keys] of Object.entries(INVALIDATION_MAP)) {
        source.addEventListener(kind, () => schedule(keys));
      }
    }

    connect();

    return () => {
      stopped = true;
      es?.close();
      if (debounceTimer) clearTimeout(debounceTimer);
      if (reconnectTimer) clearTimeout(reconnectTimer);
      setStatus("offline");
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [qc, setStatus]);
}
