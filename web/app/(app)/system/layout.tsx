import type { Metadata } from "next";

export const metadata: Metadata = { title: "System health" };

export default function Layout({ children }: { children: React.ReactNode }) {
  return children;
}
