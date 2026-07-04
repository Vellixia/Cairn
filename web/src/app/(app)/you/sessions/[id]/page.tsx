import SessionDetail from "./SessionDetail";

/**
 * Server wrapper. `generateStaticParams` returns a single placeholder so Next.js's
 * `output: "export"` pre-renders one shell page; the cairn-server static-fallback serves that
 * shell for any id the export didn't pre-render.
 *
 * BUG FIX (found alongside the same defect in `projects/[id]` - see that file's fuller note):
 * `params.id` (and even `useParams()`) always returns the literal placeholder string in this
 * foreign-server static-export setup, on both a hard reload and a client-side transition.
 * `SessionDetail` reads the real id from `window.location.pathname` instead (`useUrlId`).
 */
export function generateStaticParams() {
  return [{ id: "new" }];
}

export default function SessionDetailPage() {
  return <SessionDetail />;
}
