"use client";

import { useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Copy, KeyRound, Loader2, Plus, UserCog } from "lucide-react";
import { toast } from "sonner";
import { api, type Account } from "@/lib/api";
import { ApiErrorState } from "@/components/control-plane";
import { copyText, selectElementText } from "@/lib/clipboard";
import {
  EmptyState,
  ListSkeleton,
  PageHeader,
  formatDate,
} from "@/components/page";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
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

/**
 * The web equivalent of `cairn user` (FR-890).
 *
 * Every route behind this page takes an administrator and refuses everyone else
 * before its handler runs, so this page performs no check of its own: a member
 * who reaches it sees the server's refusal, which is the honest answer and the
 * only one that cannot drift from the actual gate (FR-892).
 *
 * The list is unpaginated because the server's is: accounts are bounded by
 * headcount, which is small by construction, and the route returns all of them
 * ordered by creation (§7).
 */
export default function AdminUsersPage() {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  // Held in component state, never refetched. The server keeps no plaintext, so
  // a reload is the moment the temporary password is gone for good.
  const [secret, setSecret] = useState<{ label: string; value: string } | null>(
    null,
  );

  const users = useQuery({
    queryKey: ["admin-users"],
    queryFn: () => api.adminUsers(),
  });

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: ["admin-users"] });

  const create = useMutation({
    mutationFn: () => api.createAdminUser(email.trim(), displayName.trim()),
    onSuccess: (account) => {
      setSecret({
        label: `Temporary password for ${account.email}`,
        value: account.temporary_password,
      });
      setOpen(false);
      setEmail("");
      setDisplayName("");
      invalidate();
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const patch = useMutation({
    mutationFn: ({
      id,
      change,
    }: {
      id: string;
      change: { role?: string; status?: string };
    }) => api.patchAdminUser(id, change),
    onSuccess: () => {
      toast.success("Account updated");
      invalidate();
    },
    // The last-admin guarantee lives in the server's transaction, so the
    // refusal that enforces it arrives here as an error message rather than as
    // a rule this page re-implements.
    onError: (err: Error) => toast.error(err.message),
  });

  const reset = useMutation({
    mutationFn: (account: Account) => api.resetAdminUserPassword(account.id),
    onSuccess: (result, account) => {
      setSecret({
        label: `Temporary password for ${account.email}`,
        value: result.temporary_password,
      });
      invalidate();
    },
    onError: (err: Error) => toast.error(err.message),
  });

  const list = users.data?.users ?? [];

  return (
    <div>
      <PageHeader
        title="Accounts"
        subtitle="Create accounts, change roles, disable access, reset a password"
      >
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogTrigger render={<Button data-testid="new-account" />}>
            <Plus />
            New account
          </DialogTrigger>
          <DialogContent>
            <form
              onSubmit={(e) => {
                e.preventDefault();
                create.mutate();
              }}
            >
              <DialogHeader>
                <DialogTitle>Create an account</DialogTitle>
                <DialogDescription>
                  A temporary password is issued once, on this screen. It
                  authenticates to the password-change route and to nothing
                  else.
                </DialogDescription>
              </DialogHeader>
              <div className="grid gap-3 py-4">
                <div className="grid gap-2">
                  <Label htmlFor="account-email">Email</Label>
                  <Input
                    id="account-email"
                    data-testid="account-email"
                    type="email"
                    value={email}
                    onChange={(e) => setEmail(e.target.value)}
                    autoFocus
                    required
                  />
                </div>
                <div className="grid gap-2">
                  <Label htmlFor="account-name">Display name</Label>
                  <Input
                    id="account-name"
                    data-testid="account-name"
                    value={displayName}
                    onChange={(e) => setDisplayName(e.target.value)}
                    required
                  />
                </div>
              </div>
              <DialogFooter>
                <DialogClose
                  render={<Button type="button" variant="outline" />}
                >
                  Cancel
                </DialogClose>
                <Button
                  type="submit"
                  data-testid="create-account"
                  disabled={create.isPending}
                >
                  {create.isPending && <Loader2 className="animate-spin" />}
                  Create account
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </PageHeader>

      {secret && (
        <RevealedSecret secret={secret} onDismiss={() => setSecret(null)} />
      )}

      {users.isLoading && <ListSkeleton rows={3} />}
      {users.error != null && <ApiErrorState error={users.error} />}

      {users.data && list.length === 0 && (
        <EmptyState
          title="No accounts"
          description="This server has no accounts, which should not be reachable — an administrator is seeded at deploy time."
        />
      )}

      {list.length > 0 && (
        <Card>
          <CardContent>
            <Table data-testid="account-table">
              <TableHeader>
                <TableRow>
                  <TableHead>Account</TableHead>
                  <TableHead>Role</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Created</TableHead>
                  <TableHead className="w-0" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {list.map((account) => (
                  <TableRow
                    key={account.id}
                    data-testid="account-row"
                    data-account-id={account.id}
                  >
                    <TableCell>
                      <div className="font-medium">{account.display_name}</div>
                      <div className="text-muted-foreground text-xs">
                        {account.email}
                      </div>
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          account.role === "admin" ? "default" : "outline"
                        }
                        data-testid="account-role"
                      >
                        {account.role}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      <Badge
                        variant={
                          account.status === "active" ? "secondary" : "outline"
                        }
                        data-testid="account-status"
                      >
                        {account.status}
                      </Badge>
                      {account.must_change_password && (
                        <Badge variant="outline" className="ml-2">
                          must change password
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="text-muted-foreground text-sm">
                      {formatDate(account.created_at)}
                    </TableCell>
                    <TableCell>
                      <div className="flex justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          data-testid="account-toggle-role"
                          disabled={patch.isPending}
                          onClick={() =>
                            patch.mutate({
                              id: account.id,
                              change: {
                                role:
                                  account.role === "admin" ? "member" : "admin",
                              },
                            })
                          }
                        >
                          <UserCog />
                          {account.role === "admin" ? "Demote" : "Promote"}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          data-testid="account-toggle-status"
                          disabled={patch.isPending}
                          onClick={() =>
                            patch.mutate({
                              id: account.id,
                              change: {
                                status:
                                  account.status === "active"
                                    ? "disabled"
                                    : "active",
                              },
                            })
                          }
                        >
                          {account.status === "active" ? "Disable" : "Enable"}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          data-testid="account-reset-password"
                          disabled={reset.isPending}
                          onClick={() => reset.mutate(account)}
                        >
                          <KeyRound />
                          Reset password
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

/**
 * The one moment a temporary password exists in a browser.
 *
 * Loud, and dismissible only by the reader: nothing re-renders it away, because
 * no route reads it back — not for the administrator who created it, not for
 * anyone.
 */
function RevealedSecret({
  secret,
  onDismiss,
}: {
  secret: { label: string; value: string };
  onDismiss: () => void;
}) {
  const [copied, setCopied] = useState(false);
  const codeRef = useRef<HTMLElement>(null);

  async function copy() {
    if (await copyText(secret.value)) {
      setCopied(true);
      toast.success("Password copied to clipboard");
      setTimeout(() => setCopied(false), 2000);
      return;
    }
    if (codeRef.current) selectElementText(codeRef.current);
    toast.error("Could not reach the clipboard — the password is selected");
  }

  return (
    <Alert className="mb-6" data-testid="revealed-password">
      <KeyRound />
      <AlertTitle>{secret.label}</AlertTitle>
      <AlertDescription className="block">
        <p className="mb-3">
          Shown once. The server stores a hash, so it cannot show it again — if
          it is lost, reset the password rather than looking it up.
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <code
            ref={codeRef}
            onClick={(e) => selectElementText(e.currentTarget)}
            data-testid="temporary-password"
            className="bg-muted min-w-0 flex-1 overflow-x-auto rounded px-2 py-1.5 font-mono text-xs break-all"
          >
            {secret.value}
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
