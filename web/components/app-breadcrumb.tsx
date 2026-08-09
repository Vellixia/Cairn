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
  memory: "Memory",
  sync: "Sync",
  tokens: "API tokens",
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

  if (segments[0] === "tokens") {
    crumbs.push({ label: SECTION_LABELS.tokens });
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

    if (section === "sessions" && segments[3]) {
      crumbs.push({ label: "Sessions", href: `/projects/${id}/sessions` });
      crumbs.push({ label: "Handoff" });
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
