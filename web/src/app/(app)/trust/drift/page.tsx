"use client";

import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemTitle,
  ItemDescription,
} from "@/components/ui/item";
import { getJSON, postJSON } from "@/lib/api";
import { toast } from "sonner";

interface DriftEvent {
  id: number;
  ts: string;
  path: string;
  risk: string;
  kind: string;
  detail: string;
  status: "pending" | "approved" | "rejected";
}

export default function DriftCenterPage() {
  const qc = useQueryClient();
  const drifts = useQuery({
    queryKey: ["guard", "drift"],
    queryFn: () => getJSON<DriftEvent[]>("/api/guard/drift"),
    refetchInterval: 5_000,
  });

  async function approve(id: number) {
    try {
      await postJSON(`/api/guard/drift/${id}/approve`, {});
      toast.success(`Approved #${id}`);
      qc.invalidateQueries({ queryKey: ["guard", "drift"] });
    } catch (e) {
      toast.error(String(e));
    }
  }

  async function reject(id: number) {
    try {
      await postJSON(`/api/guard/drift/${id}/reject`, {});
      toast(`Rejected #${id}`);
      qc.invalidateQueries({ queryKey: ["guard", "drift"] });
    } catch (e) {
      toast.error(String(e));
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">

      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Drift center</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Edits flagged by <code>verify</code> as <code>warn</code> or <code>danger</code>.
          Approve to mark the edit as expected; reject to roll back. Decisions persist in the
          session drift log so a server restart doesn&apos;t lose them.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle>Pending &amp; resolved</CardTitle>
          <CardDescription>
            {drifts.data
              ? `${drifts.data.length} event(s) . newest first`
              : "Loading..."}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {drifts.isLoading ? (
            <div className="space-y-2">

              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
              <Skeleton className="h-12 w-full" />
            </div>
          ) : drifts.data && drifts.data.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              No drift events yet --- that&apos;s a good thing.
            </p>
          ) : (
            <ul className="space-y-2">
              {drifts.data?.map((d) => (
                <Item key={d.id} variant="outline" size="sm">
                  <ItemContent>
                    <ItemTitle className="font-mono text-xs">
                      #{d.id} . {d.path}
                    </ItemTitle>
                    <ItemDescription>
                      <Badge
                        variant={
                          d.risk === "danger" ? "destructive" : "secondary"
                        }
                        className="mr-1.5 font-mono text-[10px] uppercase tracking-wider"
                      >
                        {d.risk}
                      </Badge>
                      <Badge variant="outline" className="mr-1.5 font-mono text-[10px]">
                        {d.status}
                      </Badge>
                      <span className="text-[10px] text-muted-foreground">
                        {d.detail} . {new Date(d.ts).toLocaleString()}
                      </span>
                    </ItemDescription>
                  </ItemContent>
                  {d.status === "pending" && (
                    <ItemActions>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => approve(d.id)}
                      >
                        Approve
                      </Button>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => reject(d.id)}
                      >
                        Reject
                      </Button>
                    </ItemActions>
                  )}
                </Item>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}