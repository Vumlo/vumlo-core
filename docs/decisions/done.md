# Decisions: done

Accepted and built. Short form: the full texts of ADR-0001 to ADR-0069 are in git history, for example
`git show 8052baf:docs/decisions/0036-test-database-harness.md`. Still open: [in-progress.md](in-progress.md),
[planned.md](planned.md). A new decision is a line in one of the three files; a reversal says which line it replaces.

## Foundations
- ADR-0001 — Modular monolith: one Rust binary and PostgreSQL, no Redis, broker or auth SaaS. Layout reshaped by 0065.
- ADR-0002 — One installation is one institution; no tenant id; institution settings live in the config file.
- ADR-0012 — One REST API under `/api/v1`, no client-specific endpoints, additive changes only.
- ADR-0013 — The frontend is a separate app on the public API, same site as the API behind a reverse proxy.
- ADR-0014 — The allowed core crates and their features. A new crate, version or feature needs an ADR.
- ADR-0016 — Module layout and transactions: routes → service → repo, the handler owns the transaction.
- ADR-0065 — Layered crates: server → plugins → modules → learning → org → auth → files → events → access → common.
- ADR-0066 — Optional modules behind Cargo features, built on `module-api`; plugins installed and fed by events.
- ADR-0073 — New crate `chacha20poly1305` 0.11 (RustCrypto, feature `alloc`, no default features), added to ADR-0014. Used only by `plugins::secret::{seal, open}` for secret plugin settings: XChaCha20-Poly1305, a random 24-byte nonce per seal, key = HMAC-SHA256 of `plugins.secret_key` with a purpose label, associated data = install id and setting key. The sealed blob is `0x01 || nonce || ciphertext+tag` in base64url; the first byte is the format version. Replaces the hand-made HMAC construction of ADR-0072.
- ADR-0072 — Plugin settings. A manifest declares `settings` (a subset of JSON Schema: string, number, boolean, string with `enum`, list; `default`, `description`, `secret`, scope `institution` or `course`); an administrator sets them with `vumlo-cli plugin settings`; the core stores and validates the values per install, seals secrets with `plugins.secret_key` (XChaCha20-Poly1305, ADR-0073), shows no secret to anyone, and gives the values to the plugin at `GET /plugin-api/settings` (scope `settings:read`) and tells it by `plugin.settings.changed` (ids only, sent to that plugin alone). No web form yet.
- ADR-0074 — Plugin reads and course scoping. New scopes `assignments:read`, `roster:read`, `grades:read`, `quizzes:read`, `users:contact` beside `submissions:read`, `reports:write`, `settings:read`; read-only, `no-store`, and only what a student could be shown (submitted attempts, published grades, open assignments, a quiz without its key; an address only for people in a granted course). At install and with `plugin grant --courses`, an administrator gives a plugin `all` courses or a list (`plugins.install_courses`; existing installs are backfilled `all`); `access::authorize_plugin_course` decides: scope first, then course (unknown id `404`, outside the grant `403`), a list without `course_id` covers the granted courses, and the fan-out sends a plugin only the events of its courses. Lists use `?cursor=&limit=&updated_since=` with an opaque cursor over `(time, id)`; the roster has no change time and is a snapshot. The reads are `learning::plugin_read`. Amends ADR-0066 items 11 and 14.
- ADR-0054 — Deployment: `vumlo-cli`, one config schema, build-time branding, optional modules.
- ADR-0036 — Each test gets its own database and pool (`db_test`); the harness is the `test-support` crate.

## People, roles, sign-in
- ADR-0003 — Users are separate from identities; sessions are hashed random tokens; Google links only when `email_verified`.
- ADR-0004 — Institution roles (admin, teacher, student) and course roles (teacher, assistant, student); one central policy.
- ADR-0017 — Three ways to create an account; temporary passwords; session lifetimes.
- ADR-0029 — A user has a name and a surname.
- ADR-0038 — Login hardening: sessions tied to the verified credential, counted attempts, bounded hashing.
- ADR-0046 — Self-registration, email verification and password reset by single-use tokens.
- ADR-0047 — Google sign-in by redirect with state, nonce and PKCE.
- ADR-0062 — Google sign-in for the desktop app, tied to an app challenge instead of a browser cookie.
- ADR-0067 — The password is chosen when the address is verified, not at registration.
- ADR-0041 — Trusted frontends on another origin: an exact `allowed_origins` list, off by default.

## Institution, groups, courses
- ADR-0022 — The grading scale (`twelve` or `hundred`) is an institution setting.
- ADR-0031 — `institution.mode` (school or university) is required and fixed for the life of the installation.
- ADR-0044 — Institution profile keys in the config (short name, language, time zone).
- ADR-0032 — Terms are flat; groups have one level of subgroups.
- ADR-0006 — Groups are linked to courses live; the roster is a view.
- ADR-0033 — Group links, the effective roster and per-course exclusions.
- ADR-0040 — Linking a group to a course is for the owner, a teacher and admins, not assistants.
- ADR-0005 — Courses and templates share a table; templates are deep copies.
- ADR-0019 — Publishing a course is a root action (owner, teacher, admin).
- ADR-0037 — Who is on a course: active students, former students, role changes, owner transfer.
- ADR-0048 — Course info, archiving, deletion, templates and the people of a course.

## Content, assignments, grading
- ADR-0010 — Draft and published status with `publish_at`; students see only published items (last point replaced by 0049).
- ADR-0020 — The content tree: `course_id` on every row, one level of sub-modules, dense positions.
- ADR-0025 — Block documents (the canvas): JSON blocks stored inline.
- ADR-0027 — Document types live in `api-types`; video blocks store a provider and an id.
- ADR-0068 — Formulas are drawn by KaTeX, loaded on first use.
- ADR-0049 — Unpublished revisions: working and published copies of an item; class dates; grade history.
- ADR-0008 — Assignment types are `text` plus `jsonb` config, validated by a Rust type.
- ADR-0021 — Assignments and scores: `max_points`, `is_bonus`, one row per assignment item.
- ADR-0023 — Default assignment types: `text`, `no_submission`, `file`, `form`, `quiz`.
- ADR-0050 — The `quiz` type: questions, automatic draft grades, the answer key reaches a student only after publishing.
- ADR-0051 — The `form` type: fields, exclusive options shown to students only as taken.
- ADR-0069 — File storage: the `files` crate, raw-byte upload, download through the owning item or submission.
- ADR-0011 — Submissions: drafts, attempts and deadlines (details in 0035).
- ADR-0009 — Grades have draft and published slots; students only get published.
- ADR-0015 — The gradebook is its own module (now inside `learning`).
- ADR-0028 — Gradebook grades, publishing and read models.
- ADR-0030 — The assignment-row lock covers deletes; the gradebook may ask teaching for course access.

## API conventions
- ADR-0018 — Validation errors are `422` with `errors[{field, message}]`; cursor pagination.
- ADR-0039 — Input the database cannot store is refused at the edge; busy database is `503`; unreadable bodies are `422`.
- ADR-0058 — `Idempotency-Key` on POST, as a layer around the router.
- ADR-0059 — `ETag` and `If-None-Match` on JSON GET, as a layer.
- ADR-0052 — Home read models and search, scoped by the caller's roster.
- ADR-0053 — Admin read models and reading the audit log.
- ADR-0034 — Admin writes outside a course are audited.

## Events and mail
- ADR-0045 — The outbox worker (lease, backoff, dead letters) and mail delivery.

## Web and desktop
- ADR-0042 — Web: Next.js, client-rendered, typed hooks over a generated API schema.
- ADR-0043 — Interface language: a typed catalogue (`en`, `uk`), no library.
- ADR-0057 — Query-string routes so the web app exports to the desktop build.
- ADR-0056 — Desktop client: Tauri 2, the web app in a native window, the network in Rust.
- ADR-0060 — Desktop offline store (SQLite) and send queue.
- ADR-0061 — Desktop keychain, tray, deadline notifications and deep links.
- ADR-0063 — Desktop installers are built by the institution and signed when it has certificates.
- ADR-0064 — Download links of the desktop app are set by an admin.
