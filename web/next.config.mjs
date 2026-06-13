/** @type {import('next').NextConfig} */
const nextConfig = {
  // Static export so the Rust binary can embed `out/` and serve the whole UI itself.
  output: "export",
  images: { unoptimized: true },
};

export default nextConfig;
