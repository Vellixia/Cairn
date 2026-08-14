import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";
import { Providers } from "@/components/providers";

const sans = Geist({ subsets: ["latin"], variable: "--font-geist-sans" });
const mono = Geist_Mono({ subsets: ["latin"], variable: "--font-geist-mono" });

// The root layout reads `CAIRN_API_ORIGIN` from the environment on every
// request, so one image serves any origin. That only holds if the layout is not
// prerendered at build time — a prerendered shell would freeze whatever the
// build environment had, which is the bug this replaces. Nothing here is
// statically cacheable anyway: every page is an authenticated client component.
export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  // Every tab used to read "Cairn"; sections fill the template.
  title: { default: "Cairn", template: "%s · Cairn" },
  description: "Persistent, project-aware memory for AI coding agents",
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  // Only emitted when the operator actually set one. Unset means "resolve it the
  // usual way", which is same origin in production and the loopback server under
  // `next dev` — injecting an empty string instead would override both.
  const apiOrigin = process.env.CAIRN_API_ORIGIN?.trim();

  return (
    // `suppressHydrationWarning` because next-themes writes the theme class on
    // the client before React hydrates, which is the whole point of it.
    <html lang="en" suppressHydrationWarning>
      <head>
        {/* Before any client component runs, so the first request already has it.
            JSON.stringify keeps an operator-supplied value inside the literal. */}
        {apiOrigin ? (
          <script
            dangerouslySetInnerHTML={{
              __html: `window.__CAIRN_API_ORIGIN__=${JSON.stringify(apiOrigin)};`,
            }}
          />
        ) : null}
      </head>
      <body className={`${sans.variable} ${mono.variable}`}>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
