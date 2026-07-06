"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

// Legacy URL: the Trust hub was folded into Automation (web redesign v2).
export default function TrustRedirect() {
  const router = useRouter();
  useEffect(() => {
    router.replace("/automation");
  }, [router]);
  return null;
}
