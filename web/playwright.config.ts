import { defineConfig } from "@playwright/test";

const port = 3200;
// E2E_BASE_URL points the suite at a server that is already running (the app proxied to the Rust server) instead
// of starting `pnpm dev:mock`. The people to sign in as then come from E2E_STUDENT, E2E_TEACHER, E2E_ADMIN and
// E2E_PASSWORD (e2e/routes.ts). The specs that read the mock's sample courses need a server that has them.
const external = process.env.E2E_BASE_URL;

export default defineConfig({
  testDir: "./e2e",
  timeout: 300_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  use: {
    baseURL: external ?? `http://localhost:${port}`,
    viewport: { width: 1280, height: 900 },
  },
  webServer: external
    ? undefined
    : {
        command: `pnpm dev:mock --port ${port}`,
        url: `http://localhost:${port}/login`,
        reuseExistingServer: !process.env.CI,
        timeout: 120_000,
      },
});
