"use client";

import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, KeyRound, Loader2, Plus, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { api, type CreatedToken } from "@/lib/api";
import { ConfirmButton } from "@/components/confirm-button";
import { copyText, selectElementText } from "@/lib/clipboard";
import {
  EmptyState,
  ErrorState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

export default function TokensPage() {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  // Held in component state, never refetched: the server does not keep the
  // plaintext, so a reload is the point at which it is gone for good.
  const [created, setCreated] = useState<CreatedToken | null>(null);

  const tokens = useQuery({ queryKey: ["tokens"], queryFn: () => api.tokens() });

  const create = useMutation({
    mutationFn: (tokenName: string) => api.createToken(tokenName),
    onSuccess: (token) => {
      setCreated(token);
      setOpen(false);
      setName("");
      queryClient.invalidateQueries({ queryKey: ["tokens"] });
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const revoke = useMutation({
    mutationFn: (id: string) => api.revokeToken(id),
    onSuccess: () => {
      toast.success("Token revoked");
      queryClient.invalidateQueries({ queryKey: ["tokens"] });
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const list = tokens.data?.tokens ?? [];
  const live = list.filter((t) => !t.revoked_at);

  return (
    <div>
      <PageHeader
        title="API tokens"
        subtitle="The credential cairnd carries when it syncs from your machine"
      >
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogTrigger render={<Button data-testid="new-token" />}>
            <Plus />
            New token
          </DialogTrigger>
          <DialogContent>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                create.mutate(name.trim() || "cairn daemon");
              }}
            >
              <DialogHeader>
                <DialogTitle>Create an API token</DialogTitle>
                <DialogDescription>
                  Name it after the machine that will use it, so revoking the
                  right one later is obvious.
                </DialogDescription>
              </DialogHeader>
              <div className="grid gap-2 py-4">
                <Label htmlFor="token-name">Name</Label>
                <Input
                  id="token-name"
                  data-testid="token-name"
                  placeholder="cairn daemon"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  autoFocus
                />
              </div>
              <DialogFooter>
                <DialogClose render={<Button type="button" variant="outline" />}>
                  Cancel
                </DialogClose>
                <Button
                  type="submit"
                  data-testid="create-token"
                  disabled={create.isPending}
                >
                  {create.isPending && <Loader2 className="animate-spin" />}
                  Create token
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </PageHeader>

      {created && (
        <RevealedToken token={created} onDismiss={() => setCreated(null)} />
      )}

      <Card>
        <CardHeader>
          <CardTitle>Your tokens</CardTitle>
          <CardDescription>
            {live.length} active
            {list.length !== live.length &&
              ` · ${list.length - live.length} revoked`}
          </CardDescription>
        </CardHeader>
        <CardContent>
          {tokens.isLoading && <ListSkeleton rows={2} />}
          {tokens.error != null && <ErrorState error={tokens.error} />}
          {tokens.data && list.length === 0 && (
            <EmptyState
              title="No tokens yet"
              description={
                <>
                  Create one, then run{" "}
                  <code className="bg-muted rounded px-1 py-0.5 font-mono text-xs">
                    cairn auth login --token
                  </code>{" "}
                  on the machine that should sync.
                </>
              }
            />
          )}
          {list.length > 0 && (
            <Table data-testid="token-table">
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead>Last used</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="w-0" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {list.map((t) => (
                  <TableRow key={t.id} data-testid="token-row">
                    <TableCell className="font-medium">
                      <span className="flex items-center gap-2">
                        <KeyRound className="text-muted-foreground size-3.5" />
                        {t.name}
                      </span>
                    </TableCell>
                    <TableCell className="text-muted-foreground text-sm">
                      {formatDate(t.created_at)}
                    </TableCell>
                    <TableCell className="text-muted-foreground text-sm">
                      {t.last_used_at ? formatDate(t.last_used_at) : "never"}
                    </TableCell>
                    <TableCell>
                      {t.revoked_at ? (
                        <Badge variant="outline">Revoked</Badge>
                      ) : (
                        <Badge variant="secondary">Active</Badge>
                      )}
                    </TableCell>
                    <TableCell>
                      {!t.revoked_at && (
                        <ConfirmButton
                          ariaLabel={`Revoke ${t.name}`}
                          testId="revoke-token"
                          disabled={revoke.isPending}
                          title={`Revoke ${t.name}?`}
                          description="Any machine still using this token stops syncing immediately. This cannot be undone — issue a new token instead."
                          confirmLabel="Revoke token"
                          onConfirm={() => revoke.mutate(t.id)}
                        >
                          <Trash2 className="text-destructive size-4" />
                        </ConfirmButton>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

/**
 * The one moment the plaintext exists in the browser.
 *
 * It is deliberately loud and deliberately dismissible only by the reader:
 * nothing re-renders it away, because there is no second chance to copy it.
 */
function RevealedToken({
  token,
  onDismiss,
}: {
  token: CreatedToken;
  onDismiss: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const codeRef = useRef<HTMLElement>(null);

  async function copy() {
    if (await copyText(token.token)) {
      setCopied(true);
      toast.success("Token copied to clipboard");
      setTimeout(() => setCopied(false), 2000);
      return;
    }
    // Last resort: select it so one keystroke finishes the job.
    if (codeRef.current) selectElementText(codeRef.current);
    toast.error("Could not reach the clipboard — the token is selected, press copy");
  }

  return (
    <Alert className="mb-6" data-testid="revealed-token">
      <KeyRound />
      <AlertTitle>Copy {token.name} now</AlertTitle>
      <AlertDescription className="block">
        <p className="mb-3">
          This is the only time the token is shown. The server stores a hash, so
          it cannot show it to you again.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <code
            ref={codeRef}
            onClick={(e) => selectElementText(e.currentTarget)}
            data-testid="token-plaintext"
            className="bg-muted min-w-0 flex-1 overflow-x-auto rounded px-2 py-1.5 font-mono text-xs break-all"
          >
            {token.token}
          </code>
          <Button size="sm" variant="outline" onClick={copy}>
            {copied ? <Check /> : <Copy />}
            {copied ? "Copied" : "Copy"}
          </Button>
          <Button size="sm" variant="ghost" onClick={onDismiss}>
            Done
          </Button>
        </div>
      </AlertDescription>
    </Alert>
  );
}
