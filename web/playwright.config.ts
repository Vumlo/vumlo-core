import { defineConfig } from "@playwright/test";

const port = 3200;

export default defineConfig({
  testDir: "./e2e",
  timeout: 300_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  use: {
    baseURL: `http://localhost:${port}`,
    viewport: { width: 1280, height: 900 },
  },
  webServer: {
    command: `pnpm dev:mock --port ${port}`,
    url: `http://localhost:${port}/login`,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
