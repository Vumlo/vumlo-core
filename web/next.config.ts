import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { NextConfig } from "next";

// In production the web app and the API share one site behind a reverse proxy (ADR-0013).
// In development `/api/v1` is proxied to the Rust server so the session cookie stays same-origin.
// The project used to be called Learnly; refuse the old name instead of silently using the default.
if (process.env.LEARNLY_API_ORIGIN && !process.env.VUMLO_API_ORIGIN) {
  throw new Error("LEARNLY_API_ORIGIN was found; rename it to VUMLO_API_ORIGIN");
}
const apiOrigin = process.env.VUMLO_API_ORIGIN ?? "http://127.0.0.1:8080";

// The admin console answers on its own address (ADR-0095): `VUMLO_ADMIN_HOST`, else the host of
// `server.admin_origin` that `vumlo-cli brand apply` wrote. `next dev` refuses the requests of another
// host for its own resources unless that host is named.
function adminHost(): string | undefined {
  if (process.env.VUMLO_ADMIN_HOST) return process.env.VUMLO_ADMIN_HOST;
  try {
    const generated = JSON.parse(readFileSync(join(process.cwd(), "src/generated/client-config.json"), "utf8"));
    return new URL(generated.public.server.admin_origin).host;
  } catch {
    return undefined;
  }
}
const consoleHost = adminHost();

// The desktop app loads a static export (ADR-0057). An export has no server, so no rewrites and no
// redirects: the app talks to the API through Tauri, not through this origin.
const desktop = process.env.VUMLO_TARGET === "desktop";

const withServer: NextConfig = {
  async rewrites() {
    return [{ source: "/api/v1/:path*", destination: `${apiOrigin}/api/v1/:path*` }];
  },
};

const config: NextConfig = {
  reactStrictMode: true,
  // `next dev` would otherwise write its own block into web/AGENTS.md.
  agentRules: false,
  devIndicators: false,
  allowedDevOrigins: consoleHost ? [consoleHost.split(":")[0] ?? consoleHost] : [],
  // The mock API (msw) and everything that only makes sense next to it (the sample sign-in, the fake inbox) are for
  // `pnpm dev:mock` only; without the flag each of those imports is an empty stand-in, so no build carries them.
  turbopack: {
    resolveAlias:
      process.env.NEXT_PUBLIC_API_MOCKING === "enabled"
        ? {}
        : {
            "@/mocks/StartWorker": "./src/mocks/NoWorker.tsx",
            "@/mocks/demoLogin": "./src/mocks/noDemoLogin.ts",
            "@/mocks/MockMailbox": "./src/mocks/NoMailbox.tsx",
          },
  },
  ...(desktop ? { output: "export" } : withServer),
};

export default config;
