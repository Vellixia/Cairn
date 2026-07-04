"use client";

import { useEffect, useState } from "react";

/**
 * The static export's `[id]` routes only ever pre-render one shell (a literal placeholder
 * segment). Next's App Router client runtime hydrates from that shell's embedded flight-router
 * state, which hard-codes the placeholder as the route param - `useParams()` returns the build-
 * time placeholder forever in this foreign-server (non-Next) hosting setup, never the real id,
 * on both a hard reload AND a client-side `<Link>` transition (confirmed live: `useParams()`
 * kept returning "placeholder" while `window.location.pathname` correctly showed the real URL).
 *
 * This reads the real id directly from the browser URL instead, bypassing Next's router state
 * entirely. Runs in an effect (not during render) so the server-rendered shell and the first
 * client paint stay hydration-safe; the id becomes available a tick after mount, which every
 * caller already handles via a loading skeleton.
 */
export function useUrlId(): string | null {
  const [id, setId] = useState<string | null>(null);
  useEffect(() => {
    const segments = window.location.pathname.split("/").filter(Boolean);
    const last = segments[segments.length - 1];
    setId(last ? decodeURIComponent(last) : null);
  }, []);
  return id;
}
