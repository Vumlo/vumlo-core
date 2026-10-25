# 0066. Modules and plugins

Status: Accepted
Supersedes in part: ADR-0054 (items 17, 18, 19 and 22), ADR-0055 (items 1, 2 and 5)
Amends: ADR-0014 (`hmac` becomes a workspace dependency; it was already listed), ADR-0065 (two layers are added)
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
   never on `server`, `vumlo-cli` or `plugins`.
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
    start. Changing the key or the version changes the secret, so rotation is a re-install of the secret
    in the plugin.
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

### Plugin UI (a later step, the same trust model)
17. A plugin names a slot (`assignment.tab`). The web app shows it as an `<iframe>` with
    `sandbox="allow-scripts"` and without `allow-same-origin`, so the plugin's page has an opaque origin,
    no cookie of ours and no access to our DOM. The page CSP allows `frame-src` for the installed plugin
    origins only. The frame and the host talk by `postMessage` with two message types, and the host
    accepts a message only from the frame it created. The plugin page sends its own strict CSP.

## Consequences
- The core never executes foreign code. A plugin that is down delays its events; it breaks nothing.
- The scopes are checked in `access` and in the service, so a handler cannot forget to check by reading a
  different table.
- `vumlo-config` no longer lists modules. A mistyped module name gets "unknown or not compiled in",
  without a hint about a feature.
- A module that was written against the old `ModuleDef` is rewritten. The only one is the test-only
  `dummy`, whose single migration is edited in place for the new schema name; that breaks the append-only
  rule of AGENTS section 8 for a module that never ships.
- Several handlers per event type changes `events::Handlers::merge` from "replace" to "add".
- The derived secret cannot be shown again: an administrator who lost it re-installs, or raises the
  secret version.
- Not decided here: WASM, a marketplace, signing of plugins, per-course scopes, and slots for modules.
