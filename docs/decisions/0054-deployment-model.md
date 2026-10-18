# 0054. Deployment model: `vumlo-cli`, one config schema, build-time branding, optional modules

Status: Accepted
Amends: ADR-0014 (adds four crates), ADR-0034 (the CLI that writes audit rows)
Supersedes: ADR-0044, only the accent colour (items 1 and 2: the key `accent` and its enum)
Supplements: ADR-0002, ADR-0031, ADR-0044

## Context
An installation is prepared by someone who owns a server and the institution's configuration, not by
a developer: they need to write the config, check it, apply the migrations, create the first
administrator, give the web app the institution's name and logo, and find out why something does not
work. Until now that was `vumlo-server` subcommands, a health check and a README. Three more things
pushed the decision:

- The web app showed an accent colour the institution could not really set (the site kept its tested
  accent, ADR-0044), and the favicon and the app icons had no source at all.
- The config schema lived inside the server crate, so nothing else could read it, and nothing
  separated the keys the browser may see from the keys it must never see.
- Some features (an AI agent, a code-execution module) are large, not every institution wants them,
  and one that is not used must not be in its build at all.

## Options
- Where the admin tool lives: (a) more subcommands on `vumlo-server`: one binary, but the server image
  then carries image rendering and prompt code, and "admin" and "serve" share a process model; (b) **a
  separate binary and process, `vumlo-cli`, shipped from the same repository and release**. It runs where
  the config and the database are.
- What the CLI shares with the server: (a) depend on `server`: the CLI would pull in the whole core;
  (b) copy: two schemas drift; (c) **small crates both depend on**: `vumlo-config` (the schema and
  the loader), `db` (migrations and their state, the installation record, the account value types and
  password hashing).
- How the web app gets the institution's identity: (a) a runtime endpoint: the first page waits for
  it, and the icons cannot be set that way; (b) **generated at build time** from the config into one
  directory the web build reads and nothing else.
- Where a module's migrations are recorded: (a) the one `_sqlx_migrations` table, with sqlx's
  `ignore_missing`: then a database that is newer than the binary can no longer be detected, which
  `db migrate` must refuse; (b) **one table per module**, `_sqlx_migrations_<module>`, set on that
  module's `Migrator`; the core table holds core migrations only.
- The TypeScript types of `client-config.json`: (a) `ts-rs` or another crate; (b) **a 60-line
  emitter over the JSON Schema** of the one struct, which refuses a construct it does not know.
- How the web bundle excludes a module: (a) dynamic `import()` by name: the bundler cannot tell, so
  every module is bundled; (b) **a generated `enabled.ts` with a static import for each module that
  is on**: the bundler includes exactly those.

## Decision

### The tool
1. `vumlo-cli` is a separate binary and process, built from the same workspace, shipped in the same
   image or archive as `vumlo-server`, with the same version (`[workspace.package] version`). `doctor`
   fails when the CLI, the server and the web app differ.
2. It is not a deploy orchestrator or a server configurator. It prepares configuration, generated
   assets and the database, and runs `npm run build` as a subprocess. systemd units, the Caddyfile and
   firewall rules are templates in `deploy/`, never output of the CLI.
3. Commands: `init`, `config check`, `config show [--public]`, `modules list`, `module <name> ...`,
   `brand apply`, `db status`, `db migrate`, `admin create`, `build web`, `seed --demo`, `doctor`,
   `version`. Each one that writes has `--dry-run` and is idempotent.
4. The name `vumlo-cli` is reserved for this tool. A future end-user CLI gets another name
   (ADR-0055). Nothing of it is built.

### Crates
5. `vumlo-config`: the one serde schema of `vumlo.toml` (`[institution]`, `[server]`, `[database]`,
   `[web]`, `[cli]`, `[modules]`, plus `config_version`, and the sections the server already had:
   `[log]`, `[auth]`, `[editor]`, `[mail]`), the loader, the module registry, `ClientConfig` and the
   TypeScript emitter. `db`: `MIGRATOR` and the state of any `Migrator`, the installation record, the
   CLI audit row, and `Email`, `PersonName`, `Role`, `Password`, `PasswordPolicy`, `HashedPassword`
   and the Argon2id parameters. `module-api`: the contract of an optional module. `vumlo-cli` depends
   on none of `server`, builds with `SQLX_OFFLINE=true`, and has `server` as a dev-dependency only for
   its tests.
6. New dependencies (amending ADR-0014): `schemars` 1.2 (the JSON Schema of the config structs, the
   source of the visibility marks), `image` 0.25 with `png`, `ico`, `jpeg` and `webp` (decoding the
   logo, encoding the icons), `resvg` 0.48 with no default features (rendering an SVG logo in Rust:
   no text, no embedded or external images, so a logo with text must be converted to paths), and
   `rpassword` 7 (the password prompt). All four were checked on crates.io in October 2026.

### Configuration
7. Precedence, lowest first: the defaults in the schema, `vumlo.toml`, `VUMLO__SECTION__KEY`
   environment variables, `--set section.key=value`. The file is found with `--config`, then
   `VUMLO_CONFIG`, then `./vumlo.toml`, then `../vumlo.toml`. The variables of `[cli].env_file`
   (default `.env`) are added to the environment and never override it. Only `VUMLO__*` variables are
   configuration; `VUMLO_FEATURES`, `VUMLO_API_ORIGIN` and the like are not.
8. Every key is marked in the JSON Schema (`x-visibility`) `public`, `private` or `secret`; a test
   fails on an unmarked key. Public: `server.public_origin` and `institution.name`, `short_name`,
   `language`, `time_zone`, `grading_scale`. Secret: `database.url` (read from `DATABASE_URL`),
   `auth.google.client_secret`, `mail.transport.password`, and the secrets a module declares.
9. Secrets are in the environment only. A secret in the file or in `--set`, a key the schema does not
   know (a typo, or the removed `institution.accent`), a key that looks like a secret, an origin
   or domain that cannot be right, and a `config_version` newer than the binary all stop the server and
   `config check`, which lists every problem at once. The server and the CLI read the config with
   the same function, so they refuse the same things. `Secret` shows as `***` in `Debug` and as
   `<redacted>` when serialized.
10. Every URL the installation publishes is derived from `server.public_origin` (the key keeps its name;
    it already was an origin with no path, used for CSRF and CORS). There are no keys for clients that
    do not exist.
11. Institution settings live in the config only: no table, no write endpoint. `GET /institution` and
    `GET /admin/institution` are served from the typed config as before, without `accent`.
    `institution.logo` (a path or an http(s) URL, private) is new.

### Build-time branding
12. `brand apply` writes `.vumlo/generated/` next to `vumlo.toml`, from scratch every time (staged,
    then swapped in), deterministic: `client-config.json` (the public keys, `api_url`, the version, the
    state of every module, the modules with web UI, the logo name), `client-config.d.ts`,
    `vumlo.schema.json`, `site.webmanifest`, the logo as it is, and from it `favicon.ico`, `icon-192.png`,
    `icon-512.png` and `apple-touch-icon.png`. A test ties the public marks of the schema to the
    fields of `ClientConfig` and checks that no private or secret value reaches the files.
13. The web build reads only that directory (`web/scripts/brand.mjs`) and stops when it is missing.
    There is no runtime branding endpoint. A change of the config means `brand apply`, a web rebuild
    and a restart.
14. Theming is out of scope: no `[theme]`, no tokens file, no custom CSS or fonts, no theme command.
    Universities edit `web/` themselves; that may conflict with upstream updates.

### The audit
15. Every CLI command that changes the database writes `events.audit_log` in the same transaction, with
    no actor and the source `cli` (ADR-0034): `db.migrate` (resource type `database`, the nil UUID;
    meta `{"applied": {"core": [versions], "<module>": [versions]}}`), `user.create` (as before, for
    `admin create`) and `demo.seed`. A run that changes nothing writes nothing. Commands that write
    only files (`init`, `brand apply`, `build web`) are not audited: they run before the database
    exists. `meta` holds ids and counts, never a password, a token, an email or a name.

### Optional modules
16. An optional module (ai-agent, vpl, ...) is gated twice. At build time it is its own crate, compiled
    in by a Cargo feature of the same name on `server` and `vumlo-cli`, built in one cargo invocation
    with one feature set (`VUMLO_FEATURES` in the Docker build). Default features include none. At run
    time `[modules]` (`name = true`, or `[modules.name]` with `enabled` and the module's settings,
    default off) switches a compiled-in module on.
17. A module reports as `enabled`, `disabled` (compiled in, off) or `unavailable` (not compiled in).
    The registry of known names, their features, dependencies, settings and whether they have a server
    or web part lives in `vumlo-config` whatever features are on. `config check` rejects an unknown
    name, enabling an unavailable module ("build with the Cargo feature X"), enabling a module whose
    dependencies are off, an unknown or invalid setting, and a secret in the file. `modules list`,
    `doctor` and `client-config.json` report all three states.
18. A module that is off mounts no routes (`404`, and no OpenAPI paths), starts no background work and
    loses no data: nothing deletes a module's tables.
19. A module's migrations live in its crate, run whenever the module is compiled in (so switching it on
    needs no migration) and are recorded in `_sqlx_migrations_<module>`. A binary built without a
    module therefore starts on a database that holds its tables; `doctor` warns that the data is kept,
    and fails when the CLI and the server were built with different module sets.
20. A module's settings are in `[modules.<name>]`. An endpoint is private, never reaches
    `client-config.json`, must be `https` and carry no credentials (`http` only for localhost, with
    `allow_insecure_http = true`). An API key is a secret, read from the environment variable the
    module declares. Outgoing requests go through `module-api`'s `OutboundClient`: timeouts, a response
    size limit, no logging of bodies. `doctor` checks that an enabled module has its settings and
    secret and can probe the endpoint without sending user data.
21. The web UI of a module lives in `web/src/modules/<name>/` and is imported only while the module is
    on, by a generated `enabled.ts`, so an unused module is not in the bundle.
22. The only module in this repository is `dummy`, a test-only crate behind the `dummy` feature, which
    CI enables and no release build contains. It proves the mechanism and is not documented for
    operators.

### Delivery
23. `deploy/` holds the example `compose.yaml` (PostgreSQL, core, worker, web, and `vumlo-cli` as a
    service that runs once and migrates), the Dockerfiles (offline sqlx, `VUMLO_FEATURES`), the example
    `vumlo.toml` and `env.example`, systemd units with hardening, a Caddyfile that serves the web app and
    the API under one origin, and `update.sh`. `update.sh` backs up and checks the dump, stops, migrates,
    starts and runs `doctor`, stops at the first failure, and never migrates before the dump exists.
    `./vumlo` runs the local binary or, without one, the CLI service of the compose file.
24. `vumlo-server serve --no-worker` serves requests only; `vumlo-server worker` runs the outbox and the
    workers of the modules that are on. The default `serve` still does both.

## Consequences
- The server and the CLI cannot disagree about what a config means, and a key that is added must say
  whether the browser may see it.
- Changing the institution's name, language or logo takes three steps instead of one restart: `brand
  apply`, a web build, a restart. That is the price of no runtime endpoint and no waiting first page.
- Adding or removing a module is a rebuild, not an edit. `doctor` finds the two ways to get it wrong
  (the config asks for a module the binary lacks; the CLI and the server were built differently).
- The server's own `migrate`, `create-admin` and `create-user` stay, because tests and existing
  installations use them. `vumlo-cli` is the supported way to prepare an installation.
- `server.public_origin` is not renamed. A `public_url` would have been the same value under a new name.
- The web build and `typecheck` need `brand apply` first; `dev` and `dev:mock` use a stand-in.
- The Rust-rendered SVG ignores text. A logo exported with text as text renders without it.
- Not decided here: how a module's routes authenticate (ADR-0055), and any real module.
