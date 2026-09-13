"use client";

import Link from "next/link";
import type { Reference } from "@/lib/api";
import { Badge } from "@/components/ui/badge";

/**
 * One knowledge or pattern reference, rendered whole.
 *
 * **Both parts of a knowledge reference are always shown.** A reference is a
 * domain and an id, and two domains can legitimately hold the same UUID — so an
 * id on its own names nothing, and a link that carries only the id can open the
 * wrong record. A pattern reference has no domain at all, which is a third
 * thing to say rather than a missing field to paper over with `"personal"`.
 *
 * Only a `project` knowledge reference becomes a link, because the memory
 * detail route is project-scoped: personal, team and pattern records are read
 * on the Domains screen, and inventing a URL for them here would produce a
 * link to a page that cannot answer.
 */
export function ReferenceChip({
  reference,
  projectId,
}: {
  reference: Reference;
  projectId?: string;
}) {
  const short = reference.knowledge_id.slice(0, 8);
  const linkable =
    projectId !== undefined &&
    reference.ref_kind === "knowledge" &&
    reference.domain === "project";

  const body = (
    <span
      className="inline-flex items-center gap-1.5"
      data-testid="reference"
      data-reference-key={reference.reference_key}
    >
      <Badge variant="outline" data-testid="reference-kind">
        {reference.ref_kind}
      </Badge>
      {/* A knowledge reference always prints its domain; a pattern prints that
          it has none, rather than printing nothing and reading like a knowledge
          reference whose domain was dropped. */}
      <Badge variant="secondary" data-testid="reference-domain">
        {reference.domain ?? "no domain"}
      </Badge>
      <code className="font-mono text-xs" data-testid="reference-id">
        {short}
      </code>
    </span>
  );

  if (!linkable) return body;
  return (
    <Link
      href={`/projects/${projectId}/memory/${reference.knowledge_id}`}
      className="hover:underline"
    >
      {body}
    </Link>
  );
}
