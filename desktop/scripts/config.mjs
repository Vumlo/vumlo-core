// What the installer of an institution's desktop app is made of, from what `vumlo-cli brand apply` generated
// (client-config.json). Nothing else is read: one configuration, as for the web app (ADR-0054, ADR-0056).

/** `lms.ucu.edu.ua` becomes `ua.edu.ucu.lms`: letters, digits and hyphens in dot-separated parts. */
export function identifierOf(publicOrigin) {
  const host = new URL(publicOrigin).hostname.toLowerCase();
  const parts = host
    .split(".")
    .filter(Boolean)
    .reverse()
    .map((part) => part.replace(/[^a-z0-9-]/g, "-").replace(/^-+|-+$/g, "") || "x");
  // A host with no dot (localhost) would be a one-part identifier; the platforms want a reverse-DNS name.
  return (parts.length < 2 ? ["local", ...parts] : parts).join(".");
}

/** Installers want `1.2.3`; a pre-release or `dev` keeps the numbers it has and the rest is dropped. */
export function versionOf(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)/.exec(version ?? "");
  return match ? `${match[1]}.${match[2]}.${match[3]}` : "0.0.0";
}

/** The name on the installer and the shortcut: the institution's own, without what a file name cannot hold. */
export function productNameOf(institutionName) {
  const name = (institutionName ?? "").replace(/[\\/:*?"<>|\u0000-\u001f]/g, " ").replace(/\s+/g, " ").trim();
  return name || "Vumlo";
}

const ICONS = ["32x32.png", "128x128.png", "128x128@2x.png", "icon.icns", "icon.ico"];

/** The Tauri configuration to merge over `tauri.conf.json` for this institution. */
export function tauriConfigFor(client, { frontendDist, iconDir }) {
  const productName = productNameOf(client.public.institution.name);
  return {
    productName,
    // The program's file name is the scheme, `vumlo-ucu`: unique per institution, ASCII, and what the
    // links and the installed files are named after, so two institutions' apps can share a computer.
    mainBinaryName: client.desktop.url_scheme,
    version: versionOf(client.version),
    identifier: identifierOf(client.public.server.public_origin),
    build: { frontendDist },
    plugins: { "deep-link": { desktop: { schemes: [client.desktop.url_scheme] } } },
    bundle: {
      icon: ICONS.map((name) => `${iconDir}/${name}`),
      category: "Education",
      publisher: productName,
      shortDescription: `${productName}: courses, deadlines and grades`,
      longDescription: `The desktop app of ${productName}, a Vumlo installation. Read your courses and deadlines without a connection and send your work when you are back online.`,
    },
  };
}

/** What the Rust build is told, as environment variables (build.rs). */
export function buildEnvFor(client) {
  return { VUMLO_API_URL: client.api_url, VUMLO_URL_SCHEME: client.desktop.url_scheme };
}

/** Warnings worth printing before a long build. */
export function warningsFor(client) {
  const warnings = [];
  const url = new URL(client.api_url);
  if (url.protocol !== "https:" && !["localhost", "127.0.0.1"].includes(url.hostname)) {
    warnings.push(`api_url ${client.api_url} is not https: the app would send passwords over plain HTTP.`);
  }
  if (["localhost", "127.0.0.1"].includes(url.hostname)) {
    warnings.push(`api_url ${client.api_url} points at this computer: set server.public_origin in vumlo.toml before a build you give to anyone.`);
  }
  if (client.desktop.url_scheme === "vumlo") {
    warnings.push('The URL scheme is plain "vumlo": set desktop.url_scheme (vumlo-<abbreviation>) so two institutions\' apps on one computer do not share links.');
  }
  return warnings;
}
