// After `vite build`: take out what only the student app's dev server and e2e tests use, which the console
// shares the `public/` directory with, so `dist/` holds only what the console is served with.
import { rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const dist = join(resolve(dirname(fileURLToPath(import.meta.url)), ".."), "dist");
rmSync(join(dist, "mockServiceWorker.js"), { force: true });
rmSync(join(dist, "mock-plugin"), { recursive: true, force: true });
rmSync(join(dist, "google-g.svg"), { force: true });
