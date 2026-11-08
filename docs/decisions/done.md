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
- ADR-0075 — Events for plugins. A plugin subscribes explicitly (`subscribes` in the manifest) to `assignment.created` (an assignment first becomes visible), `assignment.updated`, `assignment.deadline_changed` (always with an update), `enrollment.changed`, `quiz.attempt_submitted` and `announcement.published`, beside `submission.submitted`, `grade.published` and `plugin.settings.changed`. Each is written in the transaction of the change with ids only; a draft emits nothing; delivery carries `occurred_at` (the outbox time, kept in `plugins.deliveries`); an event that names a course is sent only to a plugin that reaches it (ADR-0074). A change of group members is the internal `group.members_changed`, which a handler of `learning` turns into `enrollment.changed` per linked course and user. Not covered: a user's status or role.
- ADR-0076 — Course announcements: `teaching.announcements`, plain text, created is published. Members read at `GET /courses/{id}/announcements`; staff and admins write (the rule of `EditContent`), edit at `PATCH /announcements/{id}` and delete; only publishing is an event. New scope `announcements:read` and `GET /plugin-api/announcements`. No web screen yet.
- ADR-0077 — Plugin reports at three levels. `reports:write` covers a report on a submitted attempt, an assignment or a course, `PUT|DELETE /plugin-api/{submissions|assignments|courses}/{id}/reports/{key}`; the key (a slug of at most 40 characters) makes it replaceable, a plugin keeps at most 20 per subject, and the course grant of ADR-0074 applies. One table, `plugins.reports`, with exactly one of three cascading keys (and a generated `subject_id` in its primary key). Staff who may see the subject read them at `GET /{submissions|assignments|courses}/{id}/plugin-reports`. Replaces `PUT /plugin-api/submissions/{id}/report` (amends ADR-0066 item 15).
- ADR-0078 — Routes of a plugin's own server, through the core. The manifest may declare `routes` (≤ 20): method (GET, POST, PUT, PATCH, DELETE), a path of plain segments or `{name}` (`{course_id}` is a course), `roles` (institution roles; an administrator only if `admin` is listed) and optional `course_roles` (teacher, assistant, student in the course of `{course_id}`; an allowed administrator is not asked for one). `ANY /api/v1/plugins/{id}/{path}` takes the person with the session the core already checks (cookie with its CSRF rule, or bearer), finds the declared route (no route: `404`), resolves the course (not visible: `404`; not in the plugin's grant: `403`), lets `access::authorize_plugin_route` decide (`403`), and forwards the call to `{base_url}{path}?{query}` with `Content-Type`, `Accept` and `Accept-Language` only and the signed `X-Vumlo-User`, `-Role`, `-Course`, `-Course-Role`, `-Timestamp`, `-Nonce` and `-Signature`: `HMAC-SHA256(secret, timestamp "." canonical)` over `vumlo-proxy`, nonce, method, request-target, user, role, course, course role and the SHA-256 of the body. No cookie or authorization header reaches the plugin; a header of the caller named `X-Vumlo-*` is dropped. Replay: a fresh UUID nonce and the time on every call; the plugin refuses a call more than 300 s off or a nonce it has seen (the example does both). Segments that could climb or escape (`.`, `..`, `%`) match nothing. The answer is the plugin's, but is served from the core's origin, so only JSON, `application/problem+json`, text, CSV, iCalendar, PDF and XLSX are passed on (`nosniff`, `Content-Security-Policy: sandbox; default-src 'none'`, `no-store`; the plugin's own cookies, CSP and caching are dropped, a `Content-Disposition` only if it is `attachment`); another type, a redirect or a `5xx` is `502`, an unreachable plugin `502`, one that takes over 30 s `504`. 2xx and 4xx are passed on with their body. Bounds: 10 MiB in, 20 MiB out, no redirects.
- ADR-0079 — Running plugins that are in use. `vumlo-cli plugin upgrade <id> [--grant ...] [--dry-run]` reads the manifest again from the stored address (the id must be the same), prints what changed (version, scopes, events, settings, routes, pages) and asks only about scopes the plugin did not ask for before; everything else follows the manifest, a grant for a scope it no longer asks for goes, stored setting values that no longer fit (setting gone, other scope, secret turned plain or the reverse, no longer meeting the spec) are removed and reported, the plugin is sent `plugin.settings.changed`, and the token, the secret, the courses and the grants it keeps are untouched; audited (`cli`). `plugin rotate-token <id>` replaces the token in one statement (the old one is `401` at once) and prints the new one once; audited. Every call to `/plugin-api` counts against an allowance of its own install (`plugins.api_burst`, `plugins.api_requests_per_second`; calls with no good token share one allowance, so garbage cannot grow the limiter or use up a real plugin's) and a call over it is `429` with `Retry-After`; it is not logged. Each other call leaves a line in `plugins.api_calls` (plugin, method, route template, answer, milliseconds: no id, query or content), kept `plugins.call_log_days` (30) days by the delivery worker and read with `vumlo-cli plugin calls <id> [--hours] [--limit]`. A plugin's writes (reports) are audited in `events.audit_log` with the source `plugin`, no actor and `meta` naming the plugin, in the transaction of the write; reads are not.
- ADR-0080 — Public routes of a plugin. A route in the manifest may say `auth = "none"` (the default is `session`, ADR-0078); then it is `GET` only, has no `roles`, `course_roles` or `{course_id}` (the manifest is refused otherwise), and `ANY /api/v1/plugins/{id}/{path}` serves it with no session, for a calendar feed that an app fetches with a URL alone. The proxy finds the install and the route first; a route that is not public (or is not there) still asks for the session first, so a stranger gets `401` as before. For a public route the core takes one call from a token bucket kept for that route (`plugins.public_route_burst` 10, `plugins.public_route_requests_per_second` 2: one allowance for everyone who calls it, since nobody is identified and an address behind a proxy is not trusted to tell callers apart; over it is `429` with `Retry-After`, and the plugin is not called). It forwards with the same allow-list of headers (no cookie, no `Authorization`, nothing named `X-Vumlo-*`), without `X-Vumlo-User`, `-Role`, `-Course`, `-Course-Role`, with `X-Vumlo-Anonymous: 1`, and the signature of ADR-0078 gets a line `session` or `none` after the target, and `-` for the user and the role of a call that is `none`, so an anonymous call and a person's call never verify as each other (`X-Vumlo-Anonymous` is a hint; what is signed decides). The answer is restricted as for any route. The plugin must check its own token: the core says only that nobody is signed in. A public route is stored in `plugins.plugin_installs.routes` as `auth: "none"` (no migration; a route stored before has none and is `session`); `plugin upgrade` shows a route that changed `auth` as removed and added (`GET /feed/{name} (public)`). Amends ADR-0078.
- ADR-0081 — What was deleted, for plugins that sync by `updated_since`. `teaching.tombstones (kind, id, course_id, deleted_at)` is written by database triggers in the transaction of the delete, whatever deletes the row (an item, its module, the course): one for each **published** assignment (`content.items` of kind `assignment`; a draft was never shown) and one for each announcement. A plugin reads `GET /plugin-api/assignments/deleted` (scope `assignments:read`) and `GET /plugin-api/announcements/deleted` (`announcements:read`) with a required `deleted_since` (inclusive RFC 3339), `course_id`, `cursor`, `limit`, and gets `{items: [{id, course_id, deleted_at}], next_cursor}` ordered by `(deleted_at, id)`; the course grant of ADR-0074 applies (a course outside it is `403`; a course that is itself gone is not a `404`, since its deletions are the point). **Retention:** `plugins.tombstone_days` (30, 1 to 3650); the delivery worker deletes older rows about once an hour (with the call log), and a `deleted_since` older than the retention is `410`, which tells the plugin to read the list in full. `deleted_at` is the start time of the transaction, so a plugin catches up from a time a little before the last one it saw and takes an id twice without harm. **Not covered, on purpose:** submitted attempts and grades are never deleted (the guards of ADR-0028 and ADR-0035 refuse to delete an assignment or a course that holds either, and a published grade only changes), so they have no list; the roster (`GET /courses/{id}/members`) is a full snapshot, so a member who left is absent from it and the event `enrollment.changed` says when to read it; an assignment that stops being visible to students is not deleted, it stops being listed and its id is `404`.
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
