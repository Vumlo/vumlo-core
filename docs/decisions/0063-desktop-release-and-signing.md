# 0063. Desktop installers: built by the institution, signed when it has certificates

Status: Accepted
Amends: ADR-0055 (item 7), ADR-0056

## Context
The app is built from one institution's configuration: its server address, name, logo and link scheme are in the
program (ADR-0056). ADR-0055 said Vumlo publishes no release binaries. A universal installer would have to ask
for a server and would carry Vumlo's name, not the institution's.

## Decision
1. **Core publishes no installers.** The release archive carries what an institution needs to build its own:
   `desktop/` (the Rust crate, its lockfile, the build script, an installer workflow) and `docs/desktop.md`.
2. **One command from configuration to installer**: `node desktop/scripts/build.mjs`. It reads only
   `client-config.json` that `vumlo-cli brand apply` generated, as the web build does (ADR-0054): names, server,
   scheme and the 1024-pixel icon the CLI now also writes. Nothing is read from `vumlo.toml` directly.
3. **A workflow template** (`desktop/release/desktop-release.yml`) is shipped for the institution to copy,
   because the archive has no `.github/` and an institution may vendor core instead of forking it. A tag
   `desktop-v*` builds Linux (deb, AppImage), Windows (NSIS) and macOS (dmg, Apple silicon and Intel) and creates
   a **draft** release with `SHA256SUMS`. Core's own CI builds the program without the installer, so the path from
   configuration to program is tested for every change.
4. **Signing is optional and by secrets.** Windows: a base64 `.pfx` and its password; macOS: a Developer ID
   certificate, and an Apple ID with an app-specific password and a team id to notarize. A block of the workflow
   runs only when its secrets exist, and the release notes say what each installer got. Without secrets the
   installers are unsigned, and `docs/desktop.md` says honestly what people then see.
5. **No auto-update.** `tauri-plugin-updater` needs its own signing keys and a place to publish updates; that
   is an institution's infrastructure. A new installer is installed over the old one.
6. **The program is named after the scheme** (`vumlo-ucu`), unique per institution, so the apps of two
   institutions can be installed together; the identifier is the server's host reversed.
7. **Versions**: the app's version is the core release it was built from. `desktop/Cargo.toml` and
   `desktop/package.json` say the release version, and the release workflow refuses a tag where they do not.

## Consequences
- Every institution that wants the app needs a machine per system, or GitHub Actions' runners, and a
  certificate if it wants no warnings. This is the cost of an app that is theirs.
- Windows and macOS builds and the signing steps of the template run only on the institution's runners; core's
  CI builds on Linux. Those steps were written from the documentation of Tauri and GitHub Actions and have to be
  tried once by the first institution that signs.
- `ADR-0055` item 7 remains true for core: no binaries are published by this project.
