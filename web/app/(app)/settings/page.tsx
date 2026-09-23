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
  const me = useQuery({ queryKey: ["me"], queryFn: () => api.me() });
  const projects = useQuery({ queryKey: ["projects"], queryFn: () => api.projects() });
  const changePassword = useMutation({ mutationFn: () => api.changePassword(password), onSuccess: () => setPassword("") });
  const createProject = useMutation({ mutationFn: () => api.createProject({ name: projectName }), onSuccess: () => { setProjectName(""); client.invalidateQueries({ queryKey: ["projects"] }); } });
  return (
    <div className="space-y-10">
      <PageHeader title="Settings" subtitle="Account, projects, access, and server controls" />
      <section className="space-y-2"><h2 className="font-medium">Password</h2><form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); if (password) changePassword.mutate(); }}><Input type="password" value={password} onChange={(event) => setPassword(event.target.value)} placeholder="New password" aria-label="New password" /><Button type="submit">Change password</Button></form></section>
      <section className="space-y-2"><h2 className="font-medium">Projects</h2><form className="flex gap-2" onSubmit={(event) => { event.preventDefault(); if (projectName) createProject.mutate(); }}><Input value={projectName} onChange={(event) => setProjectName(event.target.value)} placeholder="Project name" aria-label="Project name" /><Button type="submit">Create project</Button></form><ul className="text-sm">{projects.data?.projects.map((project) => <li key={project.id}>{project.name}</li>)}</ul></section>
      <SettingsTokens />
      {me.data?.role === "admin" && <><SettingsUsers /><SettingsSystem /></>}
      <p className="text-muted-foreground text-sm">Privacy policy and logical import/export require server support.</p>
    </div>
  );
}
