"use client";

import { useState } from "react";
import { Plus, Trash2, Globe, ShieldCheck, Users } from "lucide-react";
import { useRegistryTrustedKeysQuery, useAddTrustedKeyMutation, useRemoveTrustedKeyMutation } from "@/lib/queries";
import type { RegistryTrustGrant } from "@/lib/api";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { HelpButton } from "@/components/HelpButton";
import { HELP } from "@/components/helpCopy";

function scopeBadge(allows: string) {
  const map: Record<string, { label: string; icon: typeof Globe }> = {
    public: { label: "Public", icon: Globe },
    team: { label: "Team", icon: Users },
    local: { label: "Local", icon: ShieldCheck },
  };
  const { label, icon: Icon } = map[allows] ?? { label: allows, icon: ShieldCheck };
  return (
    <Badge variant="outline" className="gap-1 font-mono text-[10px]">
      <Icon className="h-3 w-3" />
      {label}
    </Badge>
  );
}

function AddKeyDialog() {
  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [allows, setAllows] = useState("public");
  const [label, setLabel] = useState("");
  const add = useAddTrustedKeyMutation();

  function handleAdd() {
    add.mutate(
      { key, allows, label: label || undefined },
      {
        onSuccess: () => {
          setOpen(false);
          setKey("");
          setAllows("public");
          setLabel("");
        },
      },
    );
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button variant="default" size="sm">
          <Plus className="mr-1.5 h-4 w-4" />
          Add Key
        </Button>
      </DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add trusted key</DialogTitle>
        </DialogHeader>
        <div className="space-y-4">
          <div>
            <Label>Public key (64 hex chars)</Label>
            <Input
              placeholder="ed25519 hex public key"
              value={key}
              onChange={(e) => setKey(e.target.value)}
            />
          </div>
          <div>
            <Label>Scope</Label>
            <Select value={allows} onValueChange={setAllows}>
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="local">Local</SelectItem>
                <SelectItem value="team">Team</SelectItem>
                <SelectItem value="public">Public</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div>
            <Label>Label (optional)</Label>
            <Input
              placeholder="e.g. alice@vellixia"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
            />
          </div>
          <Button
            onClick={handleAdd}
            disabled={key.length < 64 || add.isPending}
            className="w-full"
          >
            {add.isPending ? "Adding…" : "Add"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

export default function TrustContent() {
  const keys = useRegistryTrustedKeysQuery();
  const remove = useRemoveTrustedKeyMutation();

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <div className="flex items-center justify-between">
            <CardTitle>Trusted Keys</CardTitle>
            <div className="flex items-center gap-2">
              <HelpButton content={HELP["/registry"]} />
              <AddKeyDialog />
            </div>
          </div>
        </CardHeader>
        <CardContent>
          {keys.isLoading ? (
            <Skeleton className="h-48 w-full" />
          ) : !keys.data || keys.data.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted-foreground">
              No trusted keys configured. Packs signed by unknown keys will be rejected.
            </p>
          ) : (
            <div className="overflow-x-auto rounded-md border border-line">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Key</TableHead>
                    <TableHead>Scope</TableHead>
                    <TableHead>Label</TableHead>
                    <TableHead>Granted</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {keys.data.map((k: RegistryTrustGrant) => (
                    <TableRow key={k.key}>
                      <TableCell>
                        <code className="rounded bg-muted px-1.5 py-0.5 text-[11px] font-mono">
                          {k.key.slice(0, 16)}…
                        </code>
                      </TableCell>
                      <TableCell>{scopeBadge(k.allows)}</TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {k.label ?? "—"}
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {new Date(k.granted_at).toLocaleDateString()}
                      </TableCell>
                      <TableCell className="text-right">
                        <AlertDialog>
                          <AlertDialogTrigger asChild>
                            <Button variant="ghost" size="icon">
                              <Trash2 className="h-4 w-4 text-destructive" />
                            </Button>
                          </AlertDialogTrigger>
                          <AlertDialogContent>
                            <AlertDialogHeader>
                              <AlertDialogTitle>Remove trusted key</AlertDialogTitle>
                              <AlertDialogDescription>
                                This key will no longer be accepted for signing packs.
                                {k.label && (
                                  <>
                                    {" "}Label: <strong>{k.label}</strong>
                                  </>
                                )}
                              </AlertDialogDescription>
                            </AlertDialogHeader>
                            <AlertDialogFooter>
                              <AlertDialogCancel>Cancel</AlertDialogCancel>
                              <AlertDialogAction
                                onClick={() => remove.mutate(k.key)}
                              >
                                Remove
                              </AlertDialogAction>
                            </AlertDialogFooter>
                          </AlertDialogContent>
                        </AlertDialog>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
