import type { NextConfig } from "next";

// In production the web app and the API share one site behind a reverse proxy (ADR-0013).
// In development `/api/v1` is proxied to the Rust server so the session cookie stays same-origin.
const apiOrigin = process.env.LEARNLY_API_ORIGIN ?? "http://127.0.0.1:8080";

const config: NextConfig = {
  reactStrictMode: true,
  // `next dev` would otherwise write its own block into web/AGENTS.md.
  agentRules: false,
  devIndicators: false,
  async rewrites() {
    return [{ source: "/api/v1/:path*", destination: `${apiOrigin}/api/v1/:path*` }];
  },
};

export default config;
