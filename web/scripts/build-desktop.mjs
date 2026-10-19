// The static export the desktop app loads (ADR-0057): the brand step, `next build` with VUMLO_TARGET=desktop,
// and no mock service worker in the result. The output is web/out, which desktop/ packs into the app.
import { spawnSync } from "node:child_process";
import { rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const run = (args, env = {}) => {
  const r = spawnSync(process.execPath, args, { cwd: web, stdio: "inherit", env: { ...process.env, ...env } });
  if (r.status !== 0) process.exit(r.status ?? 1);
};

run([join(web, "scripts", "brand.mjs"), ...process.argv.slice(2).filter((a) => a === "--optional")]);
rmSync(join(web, "out"), { recursive: true, force: true });
run([join(web, "node_modules", "next", "dist", "bin", "next"), "build"], { VUMLO_TARGET: "desktop" });
rmSync(join(web, "out", "mockServiceWorker.js"), { force: true });
