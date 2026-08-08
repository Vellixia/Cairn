import type { Metadata } from "next";
import Link from "next/link";
import "./globals.css";
import { Providers } from "@/components/providers";

export const metadata: Metadata = {
  title: "Cairn",
  description: "Persistent, project-aware memory for AI coding agents",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <body>
        <Providers>
          <div className="mx-auto max-w-5xl px-6 py-8">
            <Link
              href="/"
              className="mb-8 inline-block text-sm font-semibold tracking-tight"
            >
              Cairn
            </Link>
            {children}
          </div>
        </Providers>
      </body>
    </html>
  );
}
