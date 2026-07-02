import PackDetail from "./PackDetail";

export function generateStaticParams() {
  return [{ name: "new" }];
}

export default function PackDetailPage({ params }: { params: { name: string } }) {
  return <PackDetail name={params.name} />;
}
