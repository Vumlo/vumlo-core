# 0066. Modules and plugins

Status: Accepted
Supersedes in part: ADR-0054 (items 17, 18, 19 and 22), ADR-0055 (items 1, 2 and 5)
Amends: ADR-0014 (`hmac` becomes a workspace dependency; it was already listed), ADR-0057 (the address of an item may carry `&tab=<plugin id>`), ADR-0065 (two layers are added)
Supplements: ADR-0007, ADR-0045

## Context
ADR-0054 built the mechanism for optional modules and stopped there; ADR-0055 deferred plugins, widgets
and authenticated module routes because each needed a decision on trust. Two things are now wanted:

- A module that does real work, with its own routes behind sign-in, its own tables, its own events and
  its own background jobs, written by someone who owns a fork or a build.
- A way for an institution to attach a tool written in any language without building it into the
  server, for example a checker that reads a submission and writes a result back.

The first is code we compile; the second is code we do not control. They get different mechanisms.

## Options
- Module description: (a) a struct of function pointers (`ModuleDef`, today); (b) **a `Module` trait with
  a static manifest and a `register` method**, so a module says what it brings in one place and the core
  can sort modules by what they need.
- Where the module registry lives: (a) `vumlo-config`, for every build (today), so `config check` can say
  "build with feature X"; (b) **in the module**: each crate declares its manifest and settings, and
  `vumlo-config` checks `[modules]` against the manifests of the modules compiled in. A name that is not
  compiled in is reported as unknown or not compiled in, without the name of a feature.
- Module tables: (a) a schema per module, created by the module's own migration; (b) **a schema
  `mod_<id>` that the migrator creates, with its history table inside it** (`mod_<id>._sqlx_migrations`).
  The prefix keeps a module out of the core's schema names, and one `DROP SCHEMA` removes a module.
- Plugins: (a) load code into the process (WASM, a dynamic library): a sandbox and a signing story we do
  not have; (b) **an external HTTP service the core does not trust**: it gets only the events it asked
  for, and an API that checks a scope on every call.
- Webhook secret: (a) store it in plain text (breaks AGENTS section 11); (b) store it encrypted, which
  needs an AEAD crate; (c) **derive it**: `HMAC-SHA256(master key, install id and version)`. Nothing is
  stored, no crate is added, and raising the version rotates one plugin.

## Decision

### Layers
```
server, vumlo-cli
  -> plugins        the plugin installs, the delivery worker, the plugin API
  -> modules/*      optional crates, one per Cargo feature
  -> learning -> org -> auth -> events -> access -> common
module-api -> common, access, events          (and vumlo-config, beside the chain)
```
1. No crate of the chain depends on `plugins`, on a module or on `module-api`. `server` and `vumlo-cli`
   name a module in exactly one place each. A module depends on `module-api` and on the layers below it,
   never on `server`, `vumlo-cli` or `plugins`. `plugins` uses `module-api` for one thing, the bounded
   `OutboundClient` that is the one way the program makes outgoing requests.
2. A module may depend on another module only if it lists that module in `requires`, and then only on its
   public API (the re-exports of its `lib.rs`; a module has no `pub mod`). A test over `cargo metadata`
   checks all of this, and Cargo itself rejects a cycle of crates.

### Modules
3. A module implements `module_api::Module`: `manifest()` (id, version, `requires`, `permissions`, the
   spec of its settings, whether it has web UI), `migrations()`, and `register(&mut Registrar)`, which adds
   routes, event handlers and background jobs. The settings spec is in the manifest, not in `register`,
   because the configuration is checked before anything runs.
4. Registration is explicit: one list in `server`, one line per Cargo feature, with `#[cfg(feature)]`. No
   `inventory`, no `linkme`. The modules start in an order from a topological sort over `requires`, with
   ties broken by id; a missing required module and a cycle both stop the start and name the ids.
5. Routes are mounted at `/api/v1/modules/{id}/...`. A module route signs the user in with the same
   `Actor` extractor as the core: `ModuleState` implements `access::ActorSource`. A module permission is
   a name with the institution roles that hold it, declared in the manifest, and is decided by
   `access::authorize` (`Action::Module`), so `access` stays the only place that decides. Deny by default.
6. A module writes only the schema `mod_<id>`; its migrations name no other schema. Its migrator creates
   the schema and keeps its history in `mod_<id>._sqlx_migrations`. A module never migrates a core table.
   Migrations run whenever the module is compiled in, enabled or not, as before (ADR-0054 item 19).
7. A module's event handlers are added to the ones of the core. Several handlers may listen to one event
   type; the event is delivered again when any of them fails, so every handler tolerates a repeat.
8. `GET /api/v1/meta` lists the enabled modules (`id`, `version`) in start order. It needs no sign-in and
   carries nothing else.

### Plugins
9. A plugin is an external HTTP service. The core does not trust it: it receives ids only, and everything
   else it fetches through an API that checks a scope on every call.
10. **Install.** An administrator registers a base URL with `vumlo-cli plugin install`. The CLI reads
    `GET {base}/manifest` (`id`, `version`, `api`, `subscribes`, `scopes`, `ui`) with a 5 second timeout, a
    64 KiB limit and no redirects, from `https`, or `http` only for a loopback host. The manifest is
    checked against allowlists in the code (scopes, subscribable events, slots); a value outside them
    stops the install and is named. The administrator grants a subset of the scopes the plugin asked for.
    The core makes a token (`vplg_` and 32 random bytes) and stores only its SHA-256, and a row in
    `plugins.plugin_installs`. The token and the secret are shown once. The install is audited (source
    `cli`).
11. **Scopes** are `submissions:read` and `reports:write`. A new scope is a new line in the allowlist and
    an ADR note; nothing is granted by default.
12. **Core to plugin.** `POST {base}/events` with `{id, type, data}` (ids only; ADR-0007). Headers:
    `X-Vumlo-Timestamp` (Unix seconds) and `X-Vumlo-Signature: sha256=<hex>`, the HMAC-SHA256 of
    `timestamp + "." + body` under the plugin's secret. The plugin answers 2xx quickly and works after.
    A plugin rejects a timestamp that is more than 300 seconds old and compares signatures in constant
    time. A delivery is a row in `plugins.deliveries`, unique per install and event id, claimed with a
    lease like the outbox (ADR-0045) and retried after 10 s, 20 s, 40 s, up to an hour; the fifth failure
    is a dead letter. Delivery is at least once, so a plugin dedupes by event id. A disabled plugin gets
    nothing, and its pending rows wait.
13. **The secret is derived**: `base64url(HMAC-SHA256(plugins.secret_key, "vumlo-plugin:" + install id +
    ":" + secret version))`. `plugins.secret_key` is a secret key of the configuration (environment only,
    redacted in `Debug`). Without it the CLI refuses to install a plugin and the delivery worker does not
    start. Changing the key changes the secret of every plugin, so each plugin has to be given a new
    one: remove it and install it again.
14. **Plugin to core.** `/api/v1/plugin-api/*` with `Authorization: Bearer <token>`. It is a separate,
    narrow API, read-first. The extractor `PluginCaller` (in `access`, built through a `PluginSource`
    like `ActorSource`) answers 401 for an unknown or disabled token; every handler then calls
    `access::authorize_plugin(&caller, Scope)`, which answers 403 for a missing scope. A plugin sees a
    submitted attempt and never a draft (AGENTS section 6), and never `auto_scores`.
15. **Reports.** `PUT /plugin-api/submissions/{id}/report` (`reports:write`) stores one report per plugin
    and submission, `{summary, data}`, replacing an earlier one. Staff read the reports of a submission
    they may see.
16. A plugin sees no user's session, no cookie and no other plugin's report, and the plugin API never
    accepts a session.

### Plugin UI
17. **The frame.** A plugin names a slot in its manifest (`assignment.tab`: a tab on the page of an
    assignment). The web app shows the page as `<iframe sandbox="allow-scripts" referrerpolicy="no-referrer"
    allow="">`, never with `allow-same-origin`, `allow-top-navigation`, `allow-forms` or `allow-popups`. The
    page therefore has an opaque origin: no cookie of ours, no storage, no access to our DOM, no way to
    navigate the window or open another. The web app builds the frame only from a URL the core returned
    (`GET /plugin-slots`), and takes a message only from that frame (`event.source` is its window and
    `event.origin` is `"null"`).
18. **No CSP from the core.** An earlier draft of this item put the origins of the installed plugins in
    a CSP `frame-src`. The web app sends no CSP (ADR-0057 item 7: Next.js would need a nonce, which means
    dynamic rendering, against ADR-0042), and a header cannot list origins that are installed after the
    build. The boundary is the sandbox and the source of the URL. An operator who wants `frame-src` as
    well adds it in the reverse proxy (an example is commented in `deploy/Caddyfile`). **The desktop app
    shows no plugin tabs**: its CSP is fixed when it is built (`frame-src` YouTube and Vimeo only) and its
    webview never touches the network (ADR-0056).
19. **Who is looking.** The frame has no session, so a plugin cannot tell a student from a teacher by what
    it can see. `POST /assignments/{id}/plugin-launch {plugin_id}` answers `{token, expires_at}` for a
    caller who can see the assignment; unknown, disabled and slotless plugins, and an assignment the caller
    cannot see, are `404`. The token is `base64url(payload) + "." + hex`, the HMAC-SHA256, under the
    plugin's secret, of `"vumlo-launch." + base64url(payload)`. The prefix keeps it apart from the
    signature of an event (`timestamp.body`), so neither can be used for the other. The payload is
    `{v, plugin, assignment, user, role, iat, exp}` with `role` either `student` or `staff` (the caller's
    role in the course, not their institution role) and `exp` sixty seconds after `iat`. The plugin's own
    server checks it, with the secret it already has.
20. **Messages** (all carry `v: 1`). `vumlo.ready` (frame to host): the frame is listening and wants its
    context; it may send it again for a fresh token. `vumlo.context` (host to frame): `{assignment_id,
    launch: {token, expires_at}, theme, language, tokens}`. `vumlo.resize` (frame to host): `{height}`, an
    integer, clamped to 120 to 4000 pixels. Any other message is ignored. The host has to post with the
    target `"*"`, because the frame's origin is opaque, so the context holds ids, a token that lives sixty
    seconds, and colours: never a session or the token of an API.
21. **Look.** A sandboxed page cannot share the React components of the web app, and it should not.
    What it can share is the look: `tokens` is the computed value of a fixed list of design tokens
    (`--bg`, `--surface`, `--text`, `--accent`, ...) and `theme` says `light` or `dark`; the host posts
    the context again when the theme changes. The plugin draws with `var(--text)` and the others.
    Components the host draws from data a plugin returns (widgets, ADR-0055 item 1) are a design of
    their own and are not decided here.
22. **The plugin's own page** sends a strict CSP of its own (the example hashes its inline script and
    style), and, when it wants to be framed by the institution only, `frame-ancestors` with its origin. Its
    server answers a call from the frame with CORS, because the origin of the frame is `null`.

## Consequences
- The core never executes foreign code. A plugin that is down delays its events; it breaks nothing.
- The scopes are checked in `access` and in the service, so a handler cannot forget to check by reading a
  different table.
- `vumlo-config` no longer lists modules. A name in `[modules]` that this binary does not contain, a
  typo or a module left out of the build, stops the load whether it is switched on or off, with
  "unknown, or not compiled into this binary" and the modules the build has. There is no `unavailable`
  state to report any more: `client-config.json` and `modules list` show the modules compiled in.
- The core already has routes under `/api/v1/modules/` (the modules of a course, whose ids are UUIDs).
  A module id has no hyphen, so it never equals one, and the router prefers the fixed segment. A module
  that is compiled in but off gets an explicit `404` mount, because otherwise a path such as
  `/modules/<id>/items` would reach the core route and answer `405`.
- A module that was written against the old `ModuleDef` is rewritten. The only one is the test-only
  `dummy`, whose single migration is edited in place for the new schema name; that breaks the append-only
  rule of AGENTS section 8 for a module that never ships.
- Several handlers per event type changes `events::Handlers::merge` from "replace" to "add".
- The token is stored only as a hash, so an administrator who lost it removes the plugin and installs
  it again. The secret is derived and could be shown again, but no command does: it is printed once,
  with the token. There is no command to raise `secret_version` yet.
- The tab is on a text assignment and on one that takes no submission. A quiz and a form are answered on a
  full-screen sheet (`focus`), where the web app shows no tab, so a plugin has no page there yet.
- Not decided here: WASM, a marketplace, signing of plugins, per-course scopes, slots for modules, tabs on a
  quiz or a form, and components that the web app draws from data a plugin returns.
