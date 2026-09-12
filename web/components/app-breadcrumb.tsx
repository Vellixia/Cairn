"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from "@/components/ui/breadcrumb";

const SECTION_LABELS: Record<string, string> = {
  tasks: "Tasks",
  sessions: "Sessions",
  activity: "Activity",
  memory: "Memory",
  retrievals: "Retrievals",
  agents: "Agents",
  domains: "Domains",
  sync: "Sync",
  tokens: "API tokens",
};

/** The deployment-wide screens, which hang off the root rather than a project. */
const TOP_LEVEL_LABELS: Record<string, string> = {
  tokens: "API tokens",
  team: "Team knowledge",
  system: "System health",
  admin: "Accounts",
};

type Crumb = { label: string; href?: string };

/**
 * Where you are, derived from the URL.
 *
 * The header used to print a constant "Cairn", which told a reader nothing they
 * could not see in the sidebar. A project name is looked up from the list the
 * sidebar already loads, so this costs no extra request.
 */
export function AppBreadcrumb() {
  const pathname = usePathname();
  const projects = useQuery({
    queryKey: ["projects"],
    queryFn: () => api.projects(),
  });

  const segments = pathname.split("/").filter(Boolean);
  const crumbs: Crumb[] = [{ label: "Projects", href: "/" }];

  if (segments[0] && TOP_LEVEL_LABELS[segments[0]]) {
    crumbs.push({ label: TOP_LEVEL_LABELS[segments[0]] });
  } else if (segments[0] === "projects" && segments[1]) {
    const id = segments[1];
    const name =
      projects.data?.projects.find((p) => p.id === id)?.name ?? "Project";
    const section = segments[2];
    crumbs.push(
      section
        ? { label: name, href: `/projects/${id}` }
        : { label: name },
    );

    // A third segment is a record inside a section, so the section stays in the
    // trail as a link rather than being replaced by the record.
    if (section && segments[3]) {
      const label = SECTION_LABELS[section] ?? section;
      crumbs.push({ label, href: `/projects/${id}/${section}` });
      crumbs.push({
        label:
          section === "sessions"
            ? "Handoff"
            : `${label.replace(/s$/, "")} detail`,
      });
    } else if (section) {
      crumbs.push({ label: SECTION_LABELS[section] ?? section });
    }
  }

  return (
    <Breadcrumb>
      <BreadcrumbList>
        {crumbs.map((crumb, i) => {
          const last = i === crumbs.length - 1;
          return (
            <BreadcrumbItem key={`${crumb.label}-${i}`}>
              {last || !crumb.href ? (
                <BreadcrumbPage className="max-w-40 truncate sm:max-w-none">
                  {crumb.label}
                </BreadcrumbPage>
              ) : (
                <>
                  <BreadcrumbLink
                    render={<Link href={crumb.href} />}
                    className="max-w-32 truncate sm:max-w-none"
                  >
                    {crumb.label}
                  </BreadcrumbLink>
                  <BreadcrumbSeparator />
                </>
              )}
            </BreadcrumbItem>
          );
        })}
      </BreadcrumbList>
    </Breadcrumb>
  );
}
