/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // Emit a self-contained server bundle so the container image carries the
  // app and its runtime dependencies, not the whole node_modules tree.
  output: "standalone",
};
export default nextConfig;
