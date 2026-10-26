import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Strictly typed env contract; nothing is fetched from the internet during
  // build/pages prerender — every data path is an explicit runtime endpoint.
  reactStrictMode: true,
};

export default nextConfig;
