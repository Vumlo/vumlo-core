# Architecture decisions and plan

The design is in `ARCHITECTURE.md`; one line per decision is in `docs/decisions/` ([done](decisions/done.md),
[in progress](decisions/in-progress.md), [planned](decisions/planned.md)). This file is the short version of both.
The plan below is only what those files already say; nothing in it has a date or a priority.

## Architecture decisions

- **One binary, one database.** Rust and PostgreSQL; queues, jobs and sessions live in PostgreSQL (0001).
- **One installation, one institution.** No tenant id; the mode (school or university) is fixed at first start (0002, 0031).
- **Layered crates.** Each crate depends only on the ones below it, and Cargo enforces it (0065). Optional modules and plugins sit on top (0066).
- **A module owns its schema.** It writes only there; other layers read through declared views (0016, 0065).
- **The handler owns the transaction.** One writing service call per transaction; a state change and its event commit together (0007, 0016).
- **One headless API.** `/api/v1`, additive changes only, Problem Details errors, cursor pagination (0012, 0018).
- **Authorization in one place.** `access::authorize`, deny by default (0004).
- **Students see only published data.** Draft grades, drafts and answer keys never reach them (0009, 0050).
- **Separate clients.** A Next.js web app and a Tauri desktop app use the public API only (0013, 0042, 0056).
- **Self-hosted by the institution.** `vumlo-cli` prepares config, branding and database; the institution builds its own images and installers (0054, 0063).

## Plan

Now (partly built, [in-progress.md](decisions/in-progress.md)):
- Splitting `learning` into crates (ADR-0093): the synchronous event subscribers, `content.facts`, the order of the four modules (nothing goes up) and the read views between them are built; the move into separate crates is not.
- Cleanup jobs in the scheduler: expired sessions, retention of processed outbox rows and of the audit log (`publish_at` and `auto_submit` are built, ADR-0091).
- The `link` item kind and the `lesson` item made of steps.

Next (decide first, [planned.md](decisions/planned.md)):
- Final and term grades; what the institution mode changes beyond the grading scale.
- Webhook subscriptions, CLI login with personal access tokens, `import-users`.

Later, or only if asked:
- Guardians, group ownership, an end-user CLI, mobile clients, widgets, `ai-agent` and `vpl` modules.
- Multi-tenancy, configurable roles, LTI 1.3, WASM plugins, realtime updates.
