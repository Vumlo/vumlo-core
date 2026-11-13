// The static export the desktop app loads (ADR-0057): the brand step, `next build` with VUMLO_TARGET=desktop,
// and no mock service worker in the result. The output is web/out, which desktop/ packs into the app.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, renameSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const run = (args, env = {}) => {
  const r = spawnSync(process.execPath, args, { cwd: web, stdio: "inherit", env: { ...process.env, ...env } });
  if (r.status !== 0) process.exit(r.status ?? 1);
};

run([join(web, "scripts", "brand.mjs"), ...process.argv.slice(2).filter((a) => a === "--optional")]);
rmSync(join(web, "out"), { recursive: true, force: true });
// The admin console and the proxy that picks it by host are for a server: a static export has neither
// (ADR-0095). They are put aside for the build and put back whatever its outcome.
const stash = join(web, ".desktop-stash");
const server = [join("src", "proxy.ts"), join("src", "app", "console")];
rmSync(stash, { recursive: true, force: true });
mkdirSync(stash, { recursive: true });
// `next dev` leaves types that name the pages put aside below; the build would check them and fail.
rmSync(join(web, ".next", "dev", "types"), { recursive: true, force: true });
const aside = server.filter((path) => existsSync(join(web, path)));
// `run` ends the process when the build fails, so the way back is an exit handler, not a `finally`.
process.on("exit", () => {
  aside.forEach((path, i) => existsSync(join(stash, String(i))) && renameSync(join(stash, String(i)), join(web, path)));
  rmSync(stash, { recursive: true, force: true });
});
aside.forEach((path, i) => renameSync(join(web, path), join(stash, String(i))));
run([join(web, "node_modules", "next", "dist", "bin", "next"), "build"], { VUMLO_TARGET: "desktop" });
rmSync(join(web, "out", "mockServiceWorker.js"), { force: true });
// The page of the mock plugin is for `dev:mock` and the e2e test; the desktop app shows no plugin pages (ADR-0066).
rmSync(join(web, "out", "mock-plugin"), { recursive: true, force: true });
