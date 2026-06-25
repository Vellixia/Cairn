/** @type {import('next').NextConfig} */
const nextConfig = {
  // Static export so the Rust binary can embed `out/` and serve the whole UI itself.
  output: "export",
  images: { unoptimized: true },
  // Dev-only proxy: forward /api/* to the Cairn backend so `next dev` works without
  // setting NEXT_PUBLIC_CAIRN_API. Rewrites are ignored in the static export build.
  ...(process.env.NODE_ENV === "development" && {
    async rewrites() {
      const backend =
        process.env.NEXT_PUBLIC_CAIRN_API ?? "http://127.0.0.1:7777";
      return [
        { source: "/api/:path*", destination: `${backend}/api/:path*` },
      ];
    },
  }),
};

export default nextConfig;
