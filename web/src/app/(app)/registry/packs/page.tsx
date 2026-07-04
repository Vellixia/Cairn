"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

// Legacy URL: the registry web pages were removed (web redesign v2).
export default function RegistryPacksRedirect() {
  const router = useRouter();
  useEffect(() => {
    router.replace("/");
  }, [router]);
  return null;
}
