import type { NextConfig } from "next";

// In production the web app and the API share one site behind a reverse proxy (ADR-0013).
// In development `/api/v1` is proxied to the Rust server so the session cookie stays same-origin.
// The project used to be called Learnly; refuse the old name instead of silently using the default.
if (process.env.LEARNLY_API_ORIGIN && !process.env.VUMLO_API_ORIGIN) {
  throw new Error("LEARNLY_API_ORIGIN was found; rename it to VUMLO_API_ORIGIN");
}
const apiOrigin = process.env.VUMLO_API_ORIGIN ?? "http://127.0.0.1:8080";

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
