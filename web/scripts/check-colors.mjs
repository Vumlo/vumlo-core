// Fails when a colour literal appears outside styles/tokens.css, so every colour is a theme token.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = new URL("../src", import.meta.url).pathname;
const allowed = new Set(["styles/tokens.css"]);
const literal = /#[0-9a-fA-F]{3,8}\b|\b(?:rgb|rgba|hsl|hsla)\(/;
const bad = [];

function walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path);
    else if (/\.(css|tsx?|mjs)$/.test(name) && !name.endsWith(".d.ts")) check(path);
  }
}

function check(path) {
  const rel = relative(root, path);
  if (allowed.has(rel)) return;
  readFileSync(path, "utf8")
    .split("\n")
    .forEach((line, i) => {
      // `#section` anchors and `&#39;` entities are not colours: require a hex colour shape inside a value.
      const stripped = line.replace(/href=["']#[^"']*["']/g, "").replace(/&#\d+;/g, "");
      if (literal.test(stripped) && !/^\s*(\/\/|\*|\/\*)/.test(stripped)) bad.push(`${rel}:${i + 1}: ${line.trim()}`);
    });
}

walk(root);
if (bad.length) {
  console.error("Colour literals found outside styles/tokens.css:\n" + bad.join("\n"));
  process.exit(1);
}
