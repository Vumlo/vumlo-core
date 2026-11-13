// Sets the version of a release in every place that carries it (RELEASING.md, step 1), from one argument:
//
//   node .github/release/set-version.mjs 0.5.1        (from the repository root)
//
// backend/Cargo.toml is where the version is read from (the Release workflow, the CLI); the web app, the desktop app, their
// lockfiles and the image names of deploy/compose.yaml follow it, and the tests of vumlo-cli fail if one does not. It also
// opens the changelog entry (the heading and the compare links; the text is yours) and lets cargo update both Cargo.lock files.
// Each edit must hit exactly the number of places it expects, or nothing is written: a changed layout fails loudly.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("usage: node .github/release/set-version.mjs X.Y.Z");
  process.exit(2);
}

const read = (file) => readFileSync(root + file, "utf8");
const pending = new Map(); // file -> new text; written only when every edit has matched

/** Replaces `re` (global: every match, else the first) in `file`, which must have `count` matches. */
function edit(file, re, to, count = 1) {
  const text = pending.get(file) ?? read(file);
  const found = [...text.matchAll(new RegExp(re.source, re.flags.includes("g") ? re.flags : re.flags + "g"))].length;
  if (found !== count) throw new Error(`${file}: expected ${count} match(es) of ${re}, found ${found}`);
  pending.set(file, text.replace(re, to));
}

const current = /^\[workspace\.package\]\nversion = "([^"]+)"/m.exec(read("backend/Cargo.toml"))?.[1];
if (!current) throw new Error("backend/Cargo.toml: no [workspace.package] version");
if (current === version) {
  console.log(`already ${version}: nothing to do`);
  process.exit(0);
}

const keep = `$1${version}`; // the text before the version, then the new one
edit("backend/Cargo.toml", /^(\[workspace\.package\]\nversion = ")[^"]+/m, keep);
for (const app of ["web", "web-admin", "desktop"]) {
  edit(`${app}/package.json`, /^(  "version": ")[^"]+/m, keep);
  // package-lock.json: the package itself, twice (the top and the "" entry); a dependency named like it never matches the name line.
  edit(`${app}/package-lock.json`, new RegExp(`^(  "name": "vumlo-${app}",\\n  "version": ")[^"]+`, "m"), keep);
  edit(`${app}/package-lock.json`, new RegExp(`^(      "name": "vumlo-${app}",\\n      "version": ")[^"]+`, "m"), keep);
}
edit("desktop/Cargo.toml", /^(\[package\]\nname = "vumlo-desktop"\nversion = ")[^"]+/m, keep);
edit("deploy/compose.yaml", /(\$\{VUMLO_VERSION:-)[^}]+/g, keep, 3);

// The changelog: the entry under [Unreleased] and the compare links. The text of the entry is written by hand.
const today = new Date().toISOString().slice(0, 10);
edit("CHANGELOG.md", /^## \[Unreleased\]\n/m, () => `## [Unreleased]\n\n## [${version}] - ${today}\n`);
edit(
  "CHANGELOG.md",
  /^\[Unreleased\]: (.*)\/compare\/v[^.]+\.[^.]+\.[^.]+\.\.\.HEAD\n/m,
  (_, base) => `[Unreleased]: ${base}/compare/v${version}...HEAD\n[${version}]: ${base}/compare/v${current}...v${version}\n`,
);

for (const [file, text] of pending) writeFileSync(root + file, text);
// Cargo.lock follows the manifests. Offline: only the workspace's own packages change, nothing is fetched or upgraded.
for (const dir of ["backend", "desktop"]) execFileSync("cargo", ["update", "--workspace", "--offline"], { cwd: root + dir, stdio: "inherit" });
console.log(`${current} -> ${version} in ${[...pending.keys()].join(", ")}, and both Cargo.lock files.\nNext: write the entry in CHANGELOG.md (Upgrade notes), then the checks of RELEASING.md step 3.`);
