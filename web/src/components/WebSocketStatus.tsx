"use client";

import { useWebSocket, type WsStatus } from "@/lib/queries";
import { Badge } from "@/components/ui/badge";

function statusConfig(
  status: WsStatus,
): { label: string; dot: string; variant: "secondary" | "destructive" | "outline" } {
  switch (status) {
    case "connected":
      return { label: "Live", dot: "bg-emerald-500", variant: "secondary" };
    case "connecting":
      return { label: "Connecting...", dot: "bg-amber-500", variant: "outline" };
    case "disconnected":
      return { label: "Offline", dot: "bg-red-500", variant: "destructive" };
  }
}

export function WebSocketStatus() {
  const { status } = useWebSocket();
  const cfg = statusConfig(status);

  return (
    <Badge variant={cfg.variant} className="font-normal gap-1.5">
      <span className={`h-1.5 w-1.5 rounded-full ${cfg.dot}`} />
      <span className="hidden sm:inline">{cfg.label}</span>
    </Badge>
  );
}
