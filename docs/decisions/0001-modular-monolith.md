# 0001. Modular monolith: Rust + PostgreSQL, nothing else

Status: Accepted. Superseded in part by [ADR-0065](0065-layered-crates.md): the list of modules and the Cargo workspace.

## Context
The previous project (LearnSystem) had 6 Spring services, a gateway, Supabase, and 3 frontends for a single LMS. It was hard to run, hard to deploy, and the architecture was never chosen deliberately. The new goal is a self-hosted LMS that one institution can deploy on its own server. A secondary goal is learning Rust and backend architecture.

## Options
1. Microservices — independent deploys. But they bring network calls, distributed transactions and many containers. Overkill for one team and one institution.
2. Monolith with no internal boundaries — simplest at first, then turns into a tangle.
3. **Modular monolith** — one binary, internal modules with explicit ownership.

## Decision
- One Rust binary + PostgreSQL. No Redis, no broker, no auth SaaS.
- Queues, scheduled jobs and sessions live in PostgreSQL.
- Modules: `identity`, `org`, `teaching`, `content`, `assessment`, `files`, `events`, `policy`. Each owns its own Postgres schema.
- A module writes only its own tables. Cross-module reads via JOIN are allowed for read models.
- Layers in each module: `routes` → `service` → `repo`.
- Cargo workspace: `server`, `api-types`, `cli`.
- No library is chosen in this ADR. Each dependency gets its own ADR.

## Consequences
- Deploy = one binary + one database.
- A module can later become a separate crate without changing its API.
- Adding Redis or a broker requires a new ADR with a measured problem behind it.
