import ProjectDetail from "./ProjectDetail";

/**
 * `generateStaticParams` returns a single placeholder so Next.js's `output: "export"`
 * pre-renders one shell page (matches the `you/sessions/[id]` pattern). Real project ids are
 * loaded client-side via react-query - the cairn-server static-fallback serves this shell for
 * any id the export didn't pre-render.
 *
 * BUG FIX (found during S5 live verification): the shell is reused for every real project id,
 * on a hard reload AND a client-side `<Link>` transition alike. `params.id` is always the
 * literal placeholder string - confirmed even `useParams()` returns it forever, because Next's
 * App Router client hydrates from this shell's embedded flight-router state (baked at export
 * time) and there's no real Next.js server here to reconcile it against the actual request; the
 * foreign Rust server just serves the static file. `ProjectDetail` instead reads the id straight
 * from `window.location.pathname` (see `useUrlId`), the one source of truth confirmed correct in
 * both scenarios - so this page no longer passes `params.id` at all.
 */
export function generateStaticParams() {
  return [{ id: "placeholder" }];
}

export default function ProjectDetailPage() {
  return <ProjectDetail />;
}
