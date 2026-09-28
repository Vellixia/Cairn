"use client";

import SettingsTokens from "@/components/settings/tokens";
import SettingsSystem from "@/components/settings/system";
import SettingsUsers from "@/components/settings/users";
import { ErrorState, PageHeader } from "@/components/page";
import { ConfirmButton } from "@/components/confirm-button";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export default function SettingsPage() {
  const client = useQueryClient();
  const [password, setPassword] = useState("");
  const [projectName, setProjectName] = useState("");
  const [remote, setRemote] = useState("");
  const [projectId, setProjectId] = useState("");
  const [memberId, setMemberId] = useState("");
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.me() });
  const privacy = useQuery({ queryKey: ["privacy-policy"], queryFn: () => api.privacyPolicy() });
  const projects = useQuery({ queryKey: ["projects"], queryFn: () => api.projects() });
  const changePassword = useMutation({
    mutationFn: () => api.changePassword(password),
    onSuccess: () => setPassword(""),
  });
  const createProject = useMutation({
    mutationFn: () => api.createProject({ name: projectName.trim(), repository_remote: remote.trim() }),
    onSuccess: (project) => {
      setProjectName("");
      setRemote("");
      setProjectId(project.id);
      client.invalidateQueries({ queryKey: ["projects"] });
    },
  });
  const members = useQuery({ queryKey: ["members", projectId], queryFn: () => api.members(projectId), enabled: Boolean(projectId) });
  const addMember = useMutation({
    mutationFn: () => api.addMember(projectId, memberId.trim()),
    onSuccess: () => { setMemberId(""); client.invalidateQueries({ queryKey: ["members", projectId] }); },
  });
  const removeMember = useMutation({
    mutationFn: (userId: string) => api.removeMember(projectId, userId),
    onSuccess: () => client.invalidateQueries({ queryKey: ["members", projectId] }),
  });
  return (
    <div className="space-y-10">
      <PageHeader title="Settings" subtitle="Account, projects, access, and server controls" />
      <section className="space-y-2">
        <h2 className="font-medium">Password</h2>
        <form className="flex flex-wrap gap-2" onSubmit={(event) => { event.preventDefault(); if (!changePassword.isPending) changePassword.mutate(); }}>
          <div className="min-w-0 flex-1 space-y-1">
            <Label htmlFor="new-password">New password</Label>
            <Input id="new-password" type="password" autoComplete="new-password" minLength={8} required value={password} disabled={changePassword.isPending} onChange={(event) => { setPassword(event.target.value); changePassword.reset(); }} />
          </div>
          <Button className="self-end" type="submit" disabled={changePassword.isPending}>{changePassword.isPending ? "Changing…" : "Change password"}</Button>
        </form>
        {changePassword.error && <ErrorState error={changePassword.error} />}
        {changePassword.isSuccess && <p role="status" className="text-sm">Password changed.</p>}
      </section>
      <section className="space-y-2">
        <h2 className="font-medium">Projects</h2>
        <form className="grid gap-2 sm:grid-cols-2" onSubmit={(event) => { event.preventDefault(); if (!createProject.isPending) createProject.mutate(); }}>
          <div className="space-y-1"><Label htmlFor="project-name">Project name</Label><Input id="project-name" required value={projectName} disabled={createProject.isPending} onChange={(event) => { setProjectName(event.target.value); createProject.reset(); }} /></div>
          <div className="space-y-1"><Label htmlFor="repository-remote">Repository remote</Label><Input id="repository-remote" required value={remote} disabled={createProject.isPending} onChange={(event) => { setRemote(event.target.value); createProject.reset(); }} placeholder="git@github.com:owner/repository.git" aria-describedby="remote-help" /></div>
          <p id="remote-help" className="text-muted-foreground text-xs sm:col-span-2">Use the repository’s Git remote (HTTPS or SSH). Setup matches this remote; credentials do not belong here.</p>
          <Button type="submit" disabled={createProject.isPending}>{createProject.isPending ? "Creating…" : "Create project"}</Button>
        </form>
        {createProject.error && <ErrorState error={createProject.error} />}
        {createProject.isSuccess && <p role="status" className="text-sm">Project created. Grant access below, then create an API token and run <code>cairn setup</code> inside the matching repository.</p>}
        {projects.isLoading && <p role="status">Loading projects…</p>}
        {projects.error && <ErrorState error={projects.error} />}
        {projects.data?.projects.length === 0 && <p className="text-muted-foreground text-sm">No projects yet. Create one with its repository remote.</p>}
        <ul className="text-sm" data-testid="settings-projects">{projects.data?.projects.map((project) => <li key={project.id}><button type="button" className="underline" disabled={addMember.isPending || removeMember.isPending} onClick={() => { setProjectId(project.id); setMemberId(""); addMember.reset(); removeMember.reset(); }}>{project.name}</button></li>)}</ul>
      </section>
      {projectId && <section className="space-y-2" data-testid="project-members">
        <h2 className="font-medium">Members — {projects.data?.projects.find((project) => project.id === projectId)?.name ?? "selected project"}</h2>
        <p className="text-muted-foreground text-xs">Membership changes are authorized by the server.</p>
        <form className="flex flex-wrap gap-2" onSubmit={(event) => { event.preventDefault(); if (!addMember.isPending && !removeMember.isPending) addMember.mutate(); }}>
          <div className="min-w-0 flex-1 space-y-1"><Label htmlFor="member-id">Member user ID</Label><Input id="member-id" required value={memberId} disabled={addMember.isPending || removeMember.isPending} onChange={(event) => { setMemberId(event.target.value); addMember.reset(); }} /></div>
          <Button className="self-end" type="submit" disabled={addMember.isPending || removeMember.isPending}>{addMember.isPending ? "Adding…" : "Add member"}</Button>
        </form>
        {addMember.error && <ErrorState error={addMember.error} />}
        {removeMember.error && <ErrorState error={removeMember.error} />}
        {addMember.isSuccess && <p role="status" className="text-sm">Member added.</p>}
        {removeMember.isSuccess && <p role="status" className="text-sm">Member removed.</p>}
        {members.isLoading && <p role="status">Loading members…</p>}
        {members.error && <ErrorState error={members.error} />}
        <ul className="text-sm">{members.data?.members.map((member) => <li key={member.user_id} className="flex flex-wrap items-center gap-2">{member.display_name} <code>{member.user_id.slice(0, 8)}</code><ConfirmButton ariaLabel={`Remove ${member.display_name}`} disabled={removeMember.isPending || addMember.isPending} title={`Remove ${member.display_name}?`} description="This account will lose access to the selected project. An authorized member can grant access again." confirmLabel="Remove member" onConfirm={() => removeMember.mutate(member.user_id)}>Remove</ConfirmButton></li>)}</ul>
      </section>}
      <SettingsTokens />
      <section className="space-y-2" data-testid="privacy-policy">
        <h2 className="font-medium">Privacy policy</h2>
        {privacy.isLoading && <p role="status" className="text-muted-foreground text-sm">Loading privacy policy…</p>}
        {privacy.error && <><ErrorState error={privacy.error} /><Button type="button" variant="outline" onClick={() => privacy.refetch()} disabled={privacy.isFetching}>Retry privacy policy</Button></>}
        {privacy.data && <><p className="text-muted-foreground text-sm">Fixed by this server build; not editable.</p><ul className="list-disc pl-5 text-sm"><li>Raw observations are not stored.</li><li>All project identities for your memberships are screened.</li><li>{privacy.data.refused_field_names.length + privacy.data.refused_top_level_fields.length} payload field names are refused.</li><li>Safe-event batches: {privacy.data.batch_max_events} events, {privacy.data.body_max_bytes} bytes maximum.</li></ul></>}
      </section>
      {me.error && <ErrorState error={me.error} />}
      {me.data?.role === "admin" && <><TransferControls /><SettingsUsers /><SettingsSystem /></>}
    </div>
  );
}

function TransferControls() {
  const exportBundle = useMutation({
    mutationFn: () => api.logicalExport(),
    onSuccess: (bundle) => {
      const url = URL.createObjectURL(new Blob([JSON.stringify(bundle)], { type: "application/json" }));
      const link = document.createElement("a");
      link.href = url;
      link.download = `cairn-logical-${bundle.bundle_id}.json`;
      link.click();
      URL.revokeObjectURL(url);
    },
  });
  const importBundle = useMutation({
    mutationFn: async (file: File) => {
      if (file.size > 32 * 1024 * 1024) throw new Error("Logical import exceeds 32 MiB. Use a physical database backup for larger transfers.");
      const bundle: unknown = JSON.parse(await file.text());
      if (!bundle || typeof bundle !== "object" || !("bundle_id" in bundle) || typeof bundle.bundle_id !== "string") throw new Error("Select a Cairn logical export with a bundle_id.");
      return api.logicalImport(bundle.bundle_id, bundle);
    },
  });
  return (
    <section className="space-y-2" data-testid="logical-transfer">
      <h2 className="font-medium">Logical import/export</h2>
      <p className="text-muted-foreground text-sm">Credentials are excluded. Imported accounts start disabled; removed features remain retained offline. Logical export is not a physical database backup.</p>
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" onClick={() => exportBundle.mutate()} disabled={exportBundle.isPending || importBundle.isPending}>{exportBundle.isPending ? "Exporting…" : "Export JSON"}</Button>
        <label className="border-input bg-background inline-flex min-h-9 cursor-pointer items-center rounded-md border px-4 text-sm font-medium">
          {importBundle.isPending ? "Importing…" : "Import JSON"}
          <input className="ml-2 max-w-48 text-xs" type="file" accept="application/json,.json" disabled={importBundle.isPending || exportBundle.isPending} onChange={(event) => { const file = event.target.files?.[0]; if (file && !importBundle.isPending) importBundle.mutate(file); event.target.value = ""; }} />
        </label>
      </div>
      {exportBundle.error && <ErrorState error={exportBundle.error} />}
      {exportBundle.isSuccess && <p role="status" className="text-sm">Export downloaded.</p>}
      {importBundle.error && <ErrorState error={importBundle.error} />}
      {importBundle.data && <p role="status" className="text-sm" data-testid="import-report">{importBundle.data.accepted} accepted · {importBundle.data.unchanged} unchanged · {importBundle.data.retained} retained · {importBundle.data.rejected} rejected</p>}
    </section>
  );
}
