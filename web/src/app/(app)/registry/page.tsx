"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

// Legacy URL: the registry web pages were removed (web redesign v2). The registry itself
// is agent-facing - `cairn pack` CLI, the registry_search MCP tool, and /api/registry/*.
export default function RegistryRedirect() {
  const router = useRouter();
  useEffect(() => {
    router.replace("/");
  }, [router]);
  return null;
}
