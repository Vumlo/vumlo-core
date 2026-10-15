# 0014. Core libraries

Status: Accepted

## Context
ADR-0001 requires every dependency to be chosen deliberately. This ADR fixes the initial set. Any crate that is not listed here, or that replaces one listed here, needs a new ADR.

All crates were checked on crates.io in October 2026: they exist, are maintained and widely used. The versions below are major lines. Exact versions live in `Cargo.lock`.

## Decision

### Runtime and HTTP

| Crate | Why | Rejected |
|---|---|---|
| `tokio` 1 | The de-facto async runtime. axum, sqlx and reqwest are built on it. | — |
| `axum` 0.8 | Built by the tokio team on top of `tower`, so middleware (timeouts, tracing, CORS, rate limits) is reusable. Handlers are plain functions with typed extractors, no macros. Most used Rust web framework. | `actix-web`: equally mature, but has its own middleware system outside the `tower` ecosystem. |
| `tower-http` | Standard layers: request tracing, CORS, timeouts, body size limit, compression. | — |
| `axum-extra` | `CookieJar` for the web session cookie. | — |

### Database

| Crate | Why | Rejected |
|---|---|---|
| `sqlx` 0.9 (`postgres`, `runtime-tokio`, `tls-rustls`, `migrate`, `uuid`, `chrono`, `json`, `rust_decimal`) | We write plain SQL, and `query!` checks it against the real schema at compile time. The design relies on PostgreSQL features that ORMs hide or fight: per-module schemas, the `effective_members` view, `FOR UPDATE SKIP LOCKED` in the outbox, draft → published copied in one `UPDATE`. You see exactly what hits the database. | `sea-orm`: ORM, closest to JPA, but hides SQL and complex queries end up raw anyway. `diesel`: very strict typed query builder with a steep learning curve and a sync-first design. |
| sqlx migrations | Plain `.sql` files in `migrations/`, embedded into the binary with `sqlx::migrate!` and run by `learnly-server migrate`. One tool instead of two. | `refinery`: a separate tool for the same job. |

Consequences: CI needs either a database at build time or sqlx offline mode (`cargo sqlx prepare`, with the `.sqlx/` folder committed). Dynamic filters use `sqlx::QueryBuilder`.

### Data types

| Crate | Why |
|---|---|
| `serde` 1, `serde_json` 1 | The standard for (de)serialization. jsonb columns, API bodies, event payloads. |
| `uuid` 1 (v7) | Primary keys. v7 is time-ordered, so B-tree indexes stay compact, unlike random v4. |
| `chrono` 0.4 | Timestamps. Supported by sqlx, utoipa and serde out of the box. `jiff` has a better API but sqlx 0.9 does not support it. |
| `rust_decimal` (via sqlx) | Scores are `NUMERIC`, not floats, so 0.1 + 0.2 is exactly 0.3 in a gradebook. |

### API contract and validation

| Crate | Why | Rejected |
|---|---|---|
| `utoipa` 6 + `utoipa-axum` | OpenAPI generated from Rust types and handlers (ADR-0012). Framework-agnostic and far more widely used. | `aide`: tighter axum integration, much smaller community. |
| `utoipa-scalar` | Serves interactive API docs at `/docs`. | Swagger UI: works, but heavier and older. |
| `garde` | Declarative validation of request DTOs (`#[garde(length(max = 200))]`), supports context (e.g. allowed domains from config). | `validator`: older and more popular, less flexible. |

Rule: request DTOs are validated with `garde` at the HTTP edge. Inside services, domain values are newtypes (`Email`, `CourseTitle`) that can only be constructed valid ("parse, don't validate").

### Errors, config, logging

| Crate | Why |
|---|---|
| `thiserror` 2 | Each module has its own error enum. One `IntoResponse` impl maps errors to RFC 9457 Problem Details. |
| `anyhow` 1 | Only in `main`, CLI commands and tests, never in module APIs. |
| `config` 0.15 | `learnly.toml` + environment overrides (`LEARNLY__DATABASE__URL`). Chosen over `figment`, which has had no release since 2024. |
| `tracing` + `tracing-subscriber` | Structured logs and request spans. JSON output in production, pretty output in dev. |

### Auth and security

| Crate | Why |
|---|---|
| `argon2` 0.6 | Argon2id password hashing, the current OWASP recommendation. Stored in PHC string format, so parameters can be raised later. |
| `rand` 0.10 | 256-bit random session tokens, device codes and user codes. |
| `sha2` 0.11 | Session tokens are stored as SHA-256 hashes. Argon2 is not needed here: the tokens are random and high-entropy, unlike passwords, and a fast hash keeps the per-request lookup cheap. |
| `hmac` 0.13 | Webhook signatures (ADR-0007). |
| `openidconnect` 4 | Google sign-in: discovery, PKCE, ID-token verification for the web redirect flow and for mobile ID tokens. |
| `tower_governor` | Rate limiting of login, registration and device-code endpoints. |
| `zxcvbn` 3 | Password strength check (ADR-0017): catches common passwords, dictionary words and patterns, takes user inputs as context, and returns feedback for the UI. It replaces maintaining our own blocklist. |

Sessions and the device authorization flow are **implemented in-house** (ADR-0003). They are about 300 lines on top of the crates above. `tower-sessions` was rejected because it is cookie-centric, while we need the same session usable as a bearer token.

### Outbound, files, background work

| Crate | Why |
|---|---|
| `reqwest` 0.12 (rustls) | HTTP client for webhooks and Google. Also used by the CLI. Pinned to 0.12 because `openidconnect` 4 depends on it, so there is a single reqwest in the build instead of two. |
| `object_store` 0.14 | One trait for local disk (default) and S3-compatible storage. |
| Own worker on tokio | The outbox (ADR-0007) already is the queue. A job library such as `apalis` would add a second queue for the same job. |

### CLI

| Crate | Why |
|---|---|
| `clap` 4 (derive) | Both the server's admin subcommands and the user CLI. |
| `keyring` 4 | The user CLI stores its session token in the OS keychain, not in a plain file. |

### Testing
- `#[sqlx::test]` creates a fresh database per test and applies migrations.
- API tests start the real server on a random port and call it with `reqwest`, the same way clients do (black-box).

### Email (ADR-0017, task 004)
| Crate / tool | Why |
|---|---|
| `lettre` 0.11 | SMTP client. Emails are sent by the outbox worker, never inside a request. |
| Mailpit (Docker image, **dev only**) | Local SMTP server with a web inbox in `compose.yaml`, so emails can be checked without a real mail server. Production uses the institution's SMTP. |

### Later (separate ADRs when the feature arrives)
- A templating crate for email bodies.
- A web push / FCM client.

### Not used
ORM (`sea-orm`), `diesel`, `refinery`, Redis, a message broker, a job-queue library, JWT libraries. `diesel` and `sea-orm` were evaluated against this project's needs: cross-module transactions, views, CTE-based course copying, and code written by an LLM and reviewed by a human. `sqlx` fit best.

## Consequences
- Close to PostgreSQL. More SQL to write, but every query is visible and checked at compile time.
- Most crates are from the tokio/tower ecosystem, so they compose with each other.
- `sqlx` offline mode is part of the build setup from day one.
