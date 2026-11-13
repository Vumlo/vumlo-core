import { defineConfig } from "@playwright/test";

const port = 3201;
// E2E_BASE_URL points the suite at a console that is already running; otherwise `npm run dev:mock` is started.
const external = process.env.E2E_BASE_URL;

export default defineConfig({
  testDir: "./e2e",
  timeout: 120_000,
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
        command: `npm run dev:mock -- --port ${port}`,
        url: `http://localhost:${port}/login`,
        reuseExistingServer: !process.env.CI,
        timeout: 120_000,
      },
});
