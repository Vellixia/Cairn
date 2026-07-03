import ProjectDetail from "./ProjectDetail";

/**
 * `generateStaticParams` returns a single placeholder so Next.js's `output: "export"`
 * pre-renders one shell page (matches the `you/sessions/[id]` pattern). Real project ids are
 * loaded client-side via react-query - the cairn-server static-fallback serves this shell for
 * any id the export didn't pre-render.
 */
export function generateStaticParams() {
  return [{ id: "placeholder" }];
}

export default function ProjectDetailPage({ params }: { params: { id: string } }) {
  return <ProjectDetail id={params.id} />;
}
