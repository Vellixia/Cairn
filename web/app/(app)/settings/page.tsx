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
  const [projectId, setProjectId] = useState("");
  const [memberId, setMemberId] = useState("");
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.me() });
  const projects = useQuery({ queryKey: ["projects"], queryFn: () => api.projects() });
  const changePassword = useMutation({ mutationFn: () => api.changePassword(password), onSuccess: () => setPassword("") });
  const createProject = useMutation({ mutationFn: () => api.createProject({ name: projectName }), onSuccess: () => { setProjectName(""); client.invalidateQueries({ queryKey: ["projects"] }); } });
  const members = useQuery({ queryKey: ["members", projectId], queryFn: () => api.members(projectId), enabled: Boolean(projectId) });
  const addMember = useMutation({ mutationFn: () => api.addMember(projectId, memberId), onSuccess: () => { setMemberId(""); client.invalidateQueries({ queryKey: ["members", projectId] }); } });
  const removeMember = useMutation({ mutationFn: (userId: string) => api.removeMember(projectId, userId), onSuccess: () => client.invalidateQueries({ queryKey: ["members", projectId] }) });
  return (
    <div className="space-y-10">
      <PageHeader title="Settings" subtitle="Account, projects, access, and server controls" />
      <section className="space-y-2"><h2 className="font-medium">Password</h2><form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); if (password) changePassword.mutate(); }}><Input type="password" value={password} onChange={(event) => setPassword(event.target.value)} placeholder="New password" aria-label="New password" /><Button type="submit">Change password</Button></form></section>
      <section className="space-y-2"><h2 className="font-medium">Projects</h2><form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); if (projectName) createProject.mutate(); }}><Input value={projectName} onChange={(event) => setProjectName(event.target.value)} placeholder="Project name" aria-label="Project name" /><Button type="submit">Create project</Button></form><ul className="text-sm" data-testid="settings-projects">{projects.data?.projects.map((project) => <li key={project.id}><button type="button" className="underline" onClick={() => setProjectId(project.id)}>{project.name}</button></li>)}</ul></section>
      {projectId && <section className="space-y-2" data-testid="project-members"><h2 className="font-medium">Members</h2><p className="text-muted-foreground text-xs">Membership changes are authorized by the server.</p><form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); if (memberId) addMember.mutate(); }}><Input value={memberId} onChange={(event) => setMemberId(event.target.value)} placeholder="User ID" aria-label="Member user ID" /><Button type="submit">Add member</Button></form><ul className="text-sm">{members.data?.members.map((member) => <li key={member.user_id} className="flex items-center gap-2">{member.display_name} <code>{member.user_id.slice(0, 8)}</code><Button size="xs" variant="outline" onClick={() => removeMember.mutate(member.user_id)}>Remove</Button></li>)}</ul></section>}
      <SettingsTokens />
      {me.data?.role === "admin" && <><SettingsUsers /><SettingsSystem /></>}
      <p className="text-muted-foreground text-sm">Privacy policy and logical import/export require server support.</p>
    </div>
  );
}
