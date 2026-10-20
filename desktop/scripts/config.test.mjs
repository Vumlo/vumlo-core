import assert from "node:assert/strict";
import { test } from "node:test";
import { appNameOf, buildEnvFor, identifierOf, installerNameFor, productNameOf, tauriConfigFor, versionOf, warningsFor } from "./config.mjs";

const client = {
  version: "0.2.0",
  api_url: "https://lms.ucu.edu.ua/api/v1",
  public: { server: { public_origin: "https://lms.ucu.edu.ua" }, institution: { name: "Український католицький університет", short_name: "UCU" } },
  desktop: { url_scheme: "vumlo-ucu" },
};

test("the identifier is the host of the server, reversed", () => {
  assert.equal(identifierOf("https://lms.ucu.edu.ua"), "ua.edu.ucu.lms");
  assert.equal(identifierOf("https://LMS.Example.EDU:8443"), "edu.example.lms");
  assert.equal(identifierOf("http://localhost:3000"), "local.localhost");
  assert.equal(identifierOf("https://xn--80aa.example"), "example.xn--80aa");
});

test("the version is three numbers", () => {
  assert.equal(versionOf("0.2.0"), "0.2.0");
  assert.equal(versionOf("1.4.7-rc.1"), "1.4.7");
  assert.equal(versionOf("dev"), "0.0.0");
  assert.equal(versionOf(undefined), "0.0.0");
});

test("the product name is the institution's, safe for a file name", () => {
  assert.equal(productNameOf("Український католицький університет"), "Український католицький університет");
  assert.equal(productNameOf('A/B: "C"?'), "A B C");
  assert.equal(productNameOf("  "), "Vumlo");
});

test("the configuration for the installer carries the institution's names, scheme and icons", () => {
  const config = tauriConfigFor(client, { frontendDist: "/src/web/out", iconDir: "gen/icons", desktopTemplate: "/src/desktop/desktop-template.desktop" });
  assert.deepEqual(config.plugins["deep-link"].desktop.schemes, ["vumlo-ucu"]);
  assert.equal(config.identifier, "ua.edu.ucu.lms");
  assert.equal(config.version, "0.2.0");
  assert.equal(config.build.frontendDist, "/src/web/out");
  assert.equal(config.mainBinaryName, "vumlo-ucu", "one program name per institution");
  assert.equal(config.productName, "UCU", "the app is called by the short name");
  assert.equal(config.bundle.publisher, "Український католицький університет");
  assert.equal(config.bundle.linux.deb.desktopTemplate, "/src/desktop/desktop-template.desktop");
  assert.equal(config.bundle.category, "Education");
  assert.match(config.bundle.longDescription, /Український католицький університет/);
  assert.ok(config.bundle.icon.includes("gen/icons/icon.icns") && config.bundle.icon.includes("gen/icons/icon.ico"));
  assert.deepEqual(buildEnvFor(client), { VUMLO_API_URL: "https://lms.ucu.edu.ua/api/v1", VUMLO_URL_SCHEME: "vumlo-ucu" });
});

test("a build that would be wrong for other people is warned about", () => {
  assert.deepEqual(warningsFor(client), []);
  const local = { ...client, api_url: "http://localhost:3000/api/v1", desktop: { url_scheme: "vumlo" } };
  assert.equal(warningsFor(local).length, 2);
  const plain = { ...client, api_url: "http://lms.example.edu/api/v1" };
  assert.match(warningsFor(plain)[0], /not https/);
});

test("an installer is named after the scheme, not the institution's name", () => {
  const name = "Український католицький університет";
  assert.equal(installerNameFor(`${name}_0.2.1_amd64.deb`, name, "vumlo-ucu"), "vumlo-ucu_0.2.1_amd64.deb");
  assert.equal(installerNameFor(`${name}_0.2.1_amd64.AppImage`, name, "vumlo-ucu"), "vumlo-ucu_0.2.1_amd64.AppImage");
  assert.equal(installerNameFor(`${name}_0.2.1_x64-setup.exe`, name, "vumlo-ucu"), "vumlo-ucu_0.2.1_x64-setup.exe");
  assert.equal(installerNameFor(`${name}_0.2.1_aarch64.dmg`, name, "vumlo-ucu"), "vumlo-ucu_0.2.1_aarch64.dmg");
  assert.equal(installerNameFor("Other_0.2.1_amd64.deb", name, "vumlo-ucu"), null);
});

test("the app is called by the short name, unless a Debian package cannot be named so", () => {
  assert.equal(appNameOf({ name: "Український католицький університет", short_name: "UCU" }, "vumlo-ucu"), "UCU");
  assert.equal(appNameOf({ name: "Northfield University", short_name: "Northfield" }, "vumlo-northfield"), "Northfield");
  assert.equal(appNameOf({ name: "Український католицький університет", short_name: "УКУ" }, "vumlo-uku"), "vumlo-uku");
  assert.equal(appNameOf({ name: "Український католицький університет", short_name: "Український католицький університет" }, "vumlo-ucu"), "vumlo-ucu");
  assert.equal(appNameOf({ name: "Test", short_name: "!" }, "vumlo"), "vumlo");
});

test("the desktop file hands the address of a link to the program", async () => {
  const { readFileSync } = await import("node:fs");
  const file = readFileSync(new URL("../desktop-template.desktop", import.meta.url), "utf8");
  assert.match(file, /^Exec=\{\{exec\}\} %u$/m);
  assert.match(file, /^MimeType=\{\{mime_type\}\}$/m);
});
