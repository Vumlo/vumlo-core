# Changelog

All notable changes to Vumlo are in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the version is the one in
`backend/Cargo.toml` (`[workspace.package] version`), which the server, `vumlo-cli` and the web app share.

Before 1.0, a new minor version (`0.2.0`) may change the configuration, the API or the database
migrations; a patch version (`0.1.1`) only fixes things. Every release has **Upgrade notes**: what you
must do, in order, besides getting the new release. See [RELEASING.md](RELEASING.md).

## [Unreleased]

## [0.1.0] - 2026-10-19

The first release: a complete core for one school or university, and the tool to install it.

### Upgrade notes

There is nothing to upgrade from; follow [docs/install.md](docs/install.md).

- `config_version` is **1**. A newer value in `vumlo.toml` is refused.
- The newest core migration is `0028`. `vumlo-cli db migrate` applies the migrations; a database that
  is newer than the binary is refused. Take a backup first (`vumlo-cli db backup`, or
  `deploy/update.sh`, which does it for you).
- The institution mode (`school` or `university`) is chosen before the first start and cannot be
  changed afterwards (ADR-0031).

### Added

- **Installation** (ADR-0054, ADR-0055): `vumlo-cli`, the administrator tool: `init`, `config check`,
  `config show`, `modules list`, `brand apply`, `build web`, `db status`, `db migrate`, `db backup`,
  `admin create`, `seed --demo`, `doctor` and `version`. One configuration schema (`vumlo-config`) is
  shared by the server and the CLI; every key is `public`, `private` or `secret`, and secrets are read
  from the environment only. `doctor` fails when the CLI, the server and the web app are not one release.
- **Deployment templates** in `deploy/`: a Docker Compose file (PostgreSQL, core, worker, web), the
  Dockerfiles, `vumlo.example.toml`, `env.example`, hardened systemd units, a Caddyfile and `update.sh`
  (backup, stop, migrate, start, check). The `./vumlo` script runs the CLI from a local binary or from
  the compose service. Each release is a source archive with a checksum file, attached to its GitHub
  Release; its builds are tested from scratch in CI.
- **Placeholder brand**: the name "Vumlo" and a marked placeholder logo, `brand/logo.svg`, so that
  `init`, `brand apply` and the build work with no assets of your own. Replace the logo; `brand apply`
  generates everything from it again.
- **Branding at build time**: the institution's name, short name, language, time zone, grading scale and
  logo from `vumlo.toml` become the web app's configuration, favicon, icons and manifest
  (`brand apply`). There is no runtime branding endpoint and no theme settings.
- **Optional modules**: a Cargo feature plus `[modules]` in `vumlo.toml`. None is included in this
  release; the mechanism is tested with a test-only module.
- **Identity** (ADR-0003, ADR-0017, ADR-0038, ADR-0046, ADR-0047): password login for web and app
  clients, sessions, login hardening, self-registration limited to `auth.allowed_domains`, email
  verification, password reset and Google sign-in. Administrators create accounts with a temporary
  password that is shown once.
- **Organisation and teaching** (ADR-0005, ADR-0006, ADR-0032, ADR-0033, ADR-0037, ADR-0048):
  terms, groups and subgroups, courses and templates, group links and the live roster, course
  archiving and deletion, course owners.
- **Content** (ADR-0010, ADR-0019, ADR-0020, ADR-0024, ADR-0025, ADR-0026, ADR-0049): the content
  tree of modules with text items and assignments, block documents, self-check questions, publishing
  with dates, unpublished revisions and class dates.
- **Assessment and gradebook** (ADR-0008, ADR-0009, ADR-0011, ADR-0015, ADR-0021, ADR-0023,
  ADR-0028, ADR-0035, ADR-0050, ADR-0051): the `text`, `no_submission`, `quiz` and `form` assignment
  types, submissions with drafts, attempts and deadlines, quiz auto-scoring, draft and published
  grades, and the grade history. Students never see draft grades or answer keys.
- **Mail and events** (ADR-0007, ADR-0045): a transactional outbox and a worker that sends mail.
- **API** (ADR-0012, ADR-0018, ADR-0039, ADR-0041): everything under `/api/v1`, OpenAPI at
  `/api/v1/docs`, Problem Details errors, cursor pagination, opt-in trusted cross-origin frontends.
- **Home and admin read models** (ADR-0052, ADR-0053): course cards, the work list, search, admin
  reports and the audit log, which records every administrator action and every CLI command that
  changes the database.
- **Web app** (ADR-0042, ADR-0043): Next.js, client-rendered, with a Ukrainian and an English interface.

### Not in this release

- File uploads: the `file` assignment type and file and link items are designed (ADR-0023, ADR-0024)
  but not built.
- Any optional module, and an end-user CLI (ADR-0055).
- Published container images: the images are built on your server from the archive.

[Unreleased]: https://github.com/Vumlo/vumlo-core/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Vumlo/vumlo-core/releases/tag/v0.1.0
