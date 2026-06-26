import type { Metadata } from "next";
import "./globals.css";
import { Providers } from "./providers";

export const metadata: Metadata = {
  title: "Cairn --- dashboard",
  description:
    "Self-hosted context & reliability for AI agents. Memory, lean context, edit guardrails, and sanitized collective knowledge --- one Rust binary, no context ever lost.",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className="dark">
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
