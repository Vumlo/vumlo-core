# 0056. Desktop client: Tauri 2, the web app in a native window, the network in Rust

Status: Accepted
Amends: ADR-0012, ADR-0055 (items 4 and 7)

## Context
Students and teachers want a lightweight desktop client for one institution's server that reads offline
and keeps drafts until the connection returns. ADR-0055 left native clients undecided and said nothing
is added for clients that do not exist. This is the decision for the first one.

A spike (Tauri 2.12, Linux, WebKitGTK 2.52, the static export of a Next.js 16.4 app) found:
- The webview origin on Linux is `tauri://localhost`. For Windows and Android Tauri documents
  `http://tauri.localhost` (or `https://` when `useHttpsScheme` is on); that was **not** checked here.
- Client navigation, a hard reload on `/b?id=x`, and a direct load of a deep URL all work with
  `trailingSlash: false` (Tauri resolves `/b` to `b.html`).
- Tauri serves the **root `index.html` for a path that has no file** (it did not serve `404.html`). A
  mistyped path therefore shows the home screen, so a desktop build cannot rely on `not-found.tsx`.
- With a strict CSP, Tauri hashes the inline scripts of the exported HTML (the theme script and the
  Next.js flight scripts); the app still hydrated.

## Options
1. **Webview `fetch` with CORS and `allowed_origins`** (ADR-0041). `tauri://localhost` fails the
   http/https-only rule, the origin differs per OS and Tauri setting, and a bearer token would sit in
   JavaScript. Widening ADR-0041 for one client is a change to a security rule.
2. **`tauri-plugin-http`** (Rust-side `fetch`). No CORS, but a token still has to be handed to JavaScript
   to attach it, and the offline cache would be a second layer in front of it.
3. **Our own command `api(method, path, body)` in Rust** that makes the request, attaches the bearer
   token from the OS keychain, and is the one place that reads and writes the offline store (ADR-0060).

## Decision
1. **Option 3.** The webview never talks to the network. `desktop/src/api.rs` sends the request to the
   `api_url` of the client config (compiled in at build time), refuses any path that is not an absolute
   path inside `/api/v1` (no `..`, no scheme, no host), and returns `{status, body}`. `server.allowed_origins`
   and the API are **not changed** for the desktop client.
2. The login response's `token` stays in Rust (the keychain, ADR in step 5); the webview gets the user.
   Sessions are `client: "app"` (ADR-0003): 90 days, sliding, no CSRF Origin check.
3. `desktop/` is a Tauri project with **its own Cargo workspace and lockfile**, outside `backend/`
   (`cargo` checks of the backend do not build it). It holds no UI: the window loads `web/out`, the static
   export of `web/` (ADR-0057).
4. Dependencies, all pinned to the stable 2.x line (crates.io's newest `tauri` is a 3.0 alpha, which is
   not used):
   - Rust: `tauri` 2.12.4, `tauri-build` 2.7.3, `reqwest` 0.13 (rustls, json), `serde`, `serde_json`;
     added by later steps and listed here so they need no second ADR: `rusqlite` (bundled SQLite),
     `keyring`, `tokio`, `tauri-plugin-deep-link` 2.6.2, `tauri-plugin-single-instance` 2.5.4,
     `tauri-plugin-notification` 2.7.0, `tauri-plugin-opener` 2.7.2.
   - npm: `@tauri-apps/cli` 2.12.3 (build tool, `desktop/` only) and `@tauri-apps/api` 2.12.4 (the
     `invoke` call, a runtime dependency of `web/`, loaded only inside the desktop app).
5. The identifier of the app is derived from `server.public_origin` (the host reversed, e.g.
   `lms.ucu.edu.ua` becomes `ua.edu.ucu.lms`), the product name from `institution.name`, the version
   from the core version. The deep-link scheme is `vumlo-` plus the short name in ASCII
   (`institution.short_name`, e.g. `vumlo-ucu`); an optional public key `desktop.url_scheme`
   overrides it and must match `^vumlo-[a-z0-9][a-z0-9-]*$`. This is the first config key for a native
   client, as ADR-0055 item 4 asked for: it is marked public, because the web callback page needs the scheme.
6. **Core publishes no binaries** (ADR-0055 item 7 stays true for core). The source archive carries
   `desktop/` (without `node_modules`, `target`, `gen`) and `docs/desktop.md`. An institution builds its own
   installers; the archive includes a workflow template for that (`desktop/release/desktop-release.yml`).
7. The desktop build works only against the server it was built for (`api_url` is compiled in). One
   installation is one institution (ADR-0002).

## Consequences
- No change to CORS, CSRF or `allowed_origins` for the desktop client. Every request is IPC plus one HTTP
  call; JSON crosses the boundary twice, which is fine for this API's payloads.
- Security work is concentrated: the path check, the keychain and the store are the whole surface the
  webview can reach, and its `capabilities` file grants only our own commands.
- Windows and macOS builds, and the origins there, need a local check (the spike ran on Linux only).
- A mistyped path in the desktop app shows the home screen, not a 404. The app never builds an unknown
  path itself; deep links are mapped to known routes in Rust.
- Plain HTTP servers are only for development: the compiled-in URL is what the app trusts, and TLS is
  checked by rustls with the platform verifier.
