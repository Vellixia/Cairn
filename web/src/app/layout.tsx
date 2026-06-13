import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Cairn — context & reliability for AI agents",
  description:
    "Make any model smart. Memory, lean context, and collective knowledge for AI agents — self-hosted, one Rust binary, with no context ever lost.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
