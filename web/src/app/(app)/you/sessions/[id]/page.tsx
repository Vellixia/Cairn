import SessionDetail from "./SessionDetail";

/**
 * Server wrapper. The actual interactive UI lives in `./SessionDetail` as a
 * client component; this page just receives the dynamic route param at build
 * time and forwards it as a prop.
 *
 * `generateStaticParams` returns a single placeholder so Next.js's
 * `output: "export"` pre-renders one shell page. Real session ids are loaded
 * client-side via `react-query`, so the dashboard's
 * `/dashboard/sessions/<id>` URL works whether or not a static shell exists
 * for that exact id --- the cairn-server static-fallback serves the page if
 * Next's export hasn't pre-rendered it.
 */
export function generateStaticParams() {
  return [{ id: "new" }];
}

export default function SessionDetailPage({ params }: { params: { id: string } }) {
  return <SessionDetail id={params.id} />;
}
