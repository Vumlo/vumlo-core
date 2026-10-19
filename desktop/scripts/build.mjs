// Builds the installers of an institution's desktop app (ADR-0056, docs/desktop.md):
//
//   node scripts/build.mjs [--generated <dir>] [--skip-web] [-- <arguments for `tauri build`>]
//
// It takes the web app's static export (built here unless --skip-web), the names, scheme and server address
// from the client config that `vumlo-cli brand apply` generated, makes the installer icons from the generated
// logo, and runs `tauri build`. The generated directory is, in order: --generated, $VUMLO_GENERATED_DIR,
// ../.vumlo/generated next to vumlo.toml.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { buildEnvFor, tauriConfigFor, warningsFor } from "./config.mjs";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const web = join(desktop, "..", "web");
const args = process.argv.slice(2);
const dashes = args.indexOf("--");
const own = dashes < 0 ? args : args.slice(0, dashes);
const tauriArgs = dashes < 0 ? [] : args.slice(dashes + 1);
const option = (name) => {
  const at = own.indexOf(name);
  return at >= 0 ? own[at + 1] : undefined;
};

const generated = resolve(option("--generated") ?? process.env.VUMLO_GENERATED_DIR ?? join(desktop, "..", ".vumlo", "generated"));
const configFile = join(generated, "client-config.json");
if (!existsSync(configFile)) {
  console.error(`desktop: ${generated} has no client-config.json. Run \`vumlo-cli brand apply\` first (the build reads only that directory).`);
  process.exit(1);
}
const client = JSON.parse(readFileSync(configFile, "utf8"));
if (!client.desktop?.url_scheme) {
  console.error("desktop: client-config.json has no desktop.url_scheme: it was made by a vumlo-cli older than the desktop app. Run `vumlo-cli brand apply` again.");
  process.exit(1);
}
for (const warning of warningsFor(client)) console.warn(`desktop: warning: ${warning}`);

const run = (label, command, commandArgs, options = {}) => {
  console.log(`desktop: ${label}`);
  const result = spawnSync(command, commandArgs, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
};

if (!own.includes("--skip-web")) {
  run("building the web app for the desktop", process.execPath, [join(web, "scripts", "build-desktop.mjs")], {
    cwd: web,
    env: { ...process.env, VUMLO_GENERATED_DIR: generated },
  });
}
if (!existsSync(join(web, "out", "index.html"))) {
  console.error("desktop: web/out is empty: build the web app first (npm run build:desktop in web/).");
  process.exit(1);
}

const gen = join(desktop, "gen");
rmSync(gen, { recursive: true, force: true });
mkdirSync(gen, { recursive: true });

const tauri = join(dirname(createRequire(import.meta.url).resolve("@tauri-apps/cli/package.json")), "tauri.js");
const logo = join(generated, "icon-1024.png");
const hasLogo = existsSync(logo);
if (hasLogo) {
  run("making the installer icons from the logo", process.execPath, [tauri, "icon", logo, "--output", join(gen, "icons")], { cwd: desktop });
} else {
  console.warn("desktop: warning: institution.logo is not set, so the app has the placeholder icon of Vumlo. Set it and run `vumlo-cli brand apply`.");
}

const config = tauriConfigFor(client, { frontendDist: join(web, "out"), iconDir: hasLogo ? "gen/icons" : "icons" });
writeFileSync(join(gen, "tauri.conf.json"), JSON.stringify(config, null, 2) + "\n");

run(`building ${config.productName} ${config.version} (${config.identifier}, ${client.desktop.url_scheme}://)`, process.execPath, [tauri, "build", "--config", join(gen, "tauri.conf.json"), ...tauriArgs], {
  cwd: desktop,
  env: { ...process.env, ...buildEnvFor(client) },
});
