"use client";

import SettingsTokens from "@/components/settings/tokens";
import SettingsSystem from "@/components/settings/system";
import SettingsUsers from "@/components/settings/users";
import { PageHeader } from "@/components/page";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export default function SettingsPage() {
  const client = useQueryClient();
  const [password, setPassword] = useState("");
  const [projectName, setProjectName] = useState("");
  const [projectRemote, setProjectRemote] = useState("");
  const [projectId, setProjectId] = useState("");
  const [memberId, setMemberId] = useState("");
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.me() });
  const privacy = useQuery({ queryKey: ["privacy-policy"], queryFn: () => api.privacyPolicy() });
  const projects = useQuery({ queryKey: ["projects"], queryFn: () => api.projects() });
  const changePassword = useMutation({ mutationFn: () => api.changePassword(password), onSuccess: () => setPassword("") });
  const createProject = useMutation({ mutationFn: () => api.createProject({ name: projectName, repository_remote: projectRemote }), onSuccess: () => { setProjectName(""); setProjectRemote(""); client.invalidateQueries({ queryKey: ["projects"] }); } });
  const members = useQuery({ queryKey: ["members", projectId], queryFn: () => api.members(projectId), enabled: Boolean(projectId) });
  const addMember = useMutation({ mutationFn: () => api.addMember(projectId, memberId), onSuccess: () => { setMemberId(""); client.invalidateQueries({ queryKey: ["members", projectId] }); } });
  const removeMember = useMutation({ mutationFn: (userId: string) => api.removeMember(projectId, userId), onSuccess: () => client.invalidateQueries({ queryKey: ["members", projectId] }) });
  return (
    <div className="space-y-10">
      <PageHeader title="Settings" subtitle="Account, projects, access, and server controls" />
      <section className="space-y-2"><h2 className="font-medium">Password</h2><form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); if (password) changePassword.mutate(); }}><Input type="password" value={password} onChange={(event) => setPassword(event.target.value)} placeholder="New password" aria-label="New password" /><Button type="submit">Change password</Button></form></section>
      <section className="space-y-2"><h2 className="font-medium">Projects</h2><p className="text-muted-foreground text-xs">Repository remote must match the Git remote used by <code>cairn setup</code>.</p><form className="flex flex-wrap gap-2" onSubmit={(event) => { event.preventDefault(); if (projectName && projectRemote) createProject.mutate(); }}><Input value={projectName} onChange={(event) => setProjectName(event.target.value)} placeholder="Project name" aria-label="Project name" /><Input value={projectRemote} onChange={(event) => setProjectRemote(event.target.value)} placeholder="Repository remote" aria-label="Repository remote" /><Button type="submit" disabled={!projectName || !projectRemote || createProject.isPending}>Create project</Button></form>{createProject.error && <p className="text-destructive text-sm" role="alert">{createProject.error.message}</p>}<ul className="text-sm" data-testid="settings-projects">{projects.data?.projects.map((project) => <li key={project.id}><button type="button" className="underline" onClick={() => setProjectId(project.id)}>{project.name}</button></li>)}</ul></section>
      {projectId && <section className="space-y-2" data-testid="project-members"><h2 className="font-medium">Members</h2><p className="text-muted-foreground text-xs">Membership changes are authorized by the server.</p><form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); if (memberId) addMember.mutate(); }}><Input value={memberId} onChange={(event) => setMemberId(event.target.value)} placeholder="User ID" aria-label="Member user ID" /><Button type="submit">Add member</Button></form><ul className="text-sm">{members.data?.members.map((member) => <li key={member.user_id} className="flex items-center gap-2">{member.display_name} <code>{member.user_id.slice(0, 8)}</code><Button size="xs" variant="outline" onClick={() => removeMember.mutate(member.user_id)}>Remove</Button></li>)}</ul></section>}
      <SettingsTokens />
      <section className="space-y-2" data-testid="privacy-policy"><h2 className="font-medium">Privacy policy</h2>{privacy.data ? <><p className="text-muted-foreground text-sm">Fixed by this server build; not editable.</p><ul className="list-disc pl-5 text-sm"><li>Raw observations are not stored.</li><li>All project identities for your memberships are screened.</li><li>{privacy.data.refused_field_names.length + privacy.data.refused_top_level_fields.length} payload field names are refused.</li><li>Safe-event batches: {privacy.data.batch_max_events} events, {privacy.data.body_max_bytes} bytes maximum.</li></ul></> : <p className="text-muted-foreground text-sm">Loading privacy policy…</p>}</section>
      {me.data?.role === "admin" && <><TransferControls /><SettingsUsers /><SettingsSystem /></>}
    </div>
  );
}

function TransferControls() {
  const exportBundle = useMutation({
    mutationFn: () => api.logicalExport(),
    onSuccess: (bundle) => {
      const url = URL.createObjectURL(new Blob([JSON.stringify(bundle, null, 2)], { type: "application/json" }));
      const link = document.createElement("a");
      link.href = url;
      link.download = `cairn-logical-${bundle.bundle_id}.json`;
      link.click();
      URL.revokeObjectURL(url);
    },
  });
  const importBundle = useMutation({
    mutationFn: async (file: File) => {
      const bundle: unknown = JSON.parse(await file.text());
      const candidate = bundle as { bundle_id?: unknown };
      const importId = typeof candidate.bundle_id === "string" ? candidate.bundle_id : crypto.randomUUID();
      return api.logicalImport(importId, bundle);
    },
  });
  return (
    <section className="space-y-2" data-testid="logical-transfer">
      <h2 className="font-medium">Logical import/export</h2>
      <p className="text-muted-foreground text-sm">Credentials are excluded. Imported accounts start disabled; removed features remain retained offline.</p>
      <div className="flex flex-wrap gap-2">
        <Button type="button" variant="outline" onClick={() => exportBundle.mutate()} disabled={exportBundle.isPending}>Export JSON</Button>
        <label className="border-input bg-background inline-flex h-9 cursor-pointer items-center rounded-md border px-4 text-sm font-medium">
          Import JSON
          <input className="sr-only" type="file" accept="application/json,.json" onChange={(event) => { const file = event.target.files?.[0]; if (file) importBundle.mutate(file); event.target.value = ""; }} />
        </label>
      </div>
      {exportBundle.error && <p className="text-destructive text-sm" role="alert">{exportBundle.error.message}</p>}
      {importBundle.error && <p className="text-destructive text-sm" role="alert">{importBundle.error.message}</p>}
      {importBundle.data && <p className="text-sm" data-testid="import-report">{importBundle.data.accepted} accepted · {importBundle.data.unchanged} unchanged · {importBundle.data.retained} retained · {importBundle.data.rejected} rejected</p>}
    </section>
  );
}
