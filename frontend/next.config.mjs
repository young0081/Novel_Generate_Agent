/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  // Produce a self-contained server (.next/standalone/server.js) so the Electron
  // desktop shell can bundle and run the app without the full node_modules tree.
  output: "standalone",
};

export default nextConfig;
