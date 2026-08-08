import Link from "next/link";
import { FileText } from "lucide-react";
import type { Session } from "@/lib/api";
import { formatDate } from "@/components/page";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";

/** One vocabulary for session status everywhere it is shown. */
export function StatusBadge({ status }: { status: Session["status"] }) {
  const variant =
    status === "active"
      ? "default"
      : status === "interrupted"
        ? "destructive"
        : "secondary";
  return <Badge variant={variant}>{status}</Badge>;
}

export function SessionRow({
  session,
  projectId,
}: {
  session: Session;
  projectId: string;
}) {
  return (
    <Link href={`/projects/${projectId}/sessions/${session.id}`}>
      <Card className="hover:border-foreground/20 hover:bg-accent/40 flex-row items-center gap-3 p-3 transition">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-sm">
            <code className="font-mono text-xs">{session.branch}</code>
            <span className="text-muted-foreground">{session.agent}</span>
          </div>
          <div className="text-muted-foreground mt-0.5 text-xs">
            {formatDate(session.started_at)}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {session.has_handoff && (
            <Badge variant="outline">
              <FileText className="size-3" />
              handoff
            </Badge>
          )}
          <StatusBadge status={session.status} />
        </div>
      </Card>
    </Link>
  );
}
