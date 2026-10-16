# Learnly — Architecture

Status: **in progress.** Implemented: the foundation (config, migrations, health, OpenAPI, Problem Details), `identity` + `policy` (users, password login, sessions, admin user management), the first slice of `teaching` (courses, members, publishing) with a minimal `events.audit_log`, and the `content` tree (modules, sub-modules, lessons and `text` assignments). Everything else below is still design.
This file is the current picture of the system. Every decision behind it has a record in [`docs/decisions/`](docs/decisions/). If this file and an ADR disagree, the ADR wins and this file gets fixed.

## Goal

A self-hosted LMS core for **one school or one university per installation**.
It is headless: one HTTP API serves the web app, the mobile app and the CLI. An institution can build its own frontend on that API and extend the system through events and webhooks without forking it.

### Not in v1

- Multi-tenancy (several institutions in one installation).
- Roles configurable from the UI.
- Guardians (parents).
- Final / term grades.
- LTI 1.3, in-process (WASM) plugins.
- Realtime updates.

Each of these must stay possible to add later without a schema rewrite.

## Deployment shape

```
learnly-server (single binary)  +  PostgreSQL
```

Nothing else is required. File storage defaults to local disk, with S3-compatible storage optional. No Redis, no message broker: queues and scheduled jobs live in PostgreSQL ([ADR-0001](docs/decisions/0001-modular-monolith.md)).

## Clients

```
web (SPA)      ──cookie──┐
mobile app     ──bearer──┼──>  HTTP /api/v1  ──>  modules  ──>  PostgreSQL
learnly (CLI)  ──bearer──┘
learnly-server admin subcommands (on the server)  ──────────>  PostgreSQL
```

- **One API for every client.** No client-specific endpoints ([ADR-0012](docs/decisions/0012-api-for-multiple-clients.md)).
- **`learnly-server`** = the server binary. Subcommands: `serve`, `migrate`, `create-admin`, `create-user` (`import-users` is planned). Admin subcommands talk to the database directly.
- **`learnly`** = the user CLI for students and teachers. A normal API client with a token; it never touches the database. (Naming follows the `dockerd` / `docker` pattern: the short name goes to the command people type most.)

## Repository layout

```
crates/
  server/      # library + binary `learnly-server`: HTTP API, modules, worker, admin subcommands; build.rs
  api-types/   # request/response types shared with the Rust CLI; OpenAPI source
  cli/         # binary `learnly`: user CLI (API client)
migrations/
docs/decisions/
```

**Migrations** are plain `.sql` files in `migrations/`, append-only, embedded into `learnly-server` at compile time with `sqlx::migrate!` and applied by `learnly-server migrate`. `serve` refuses to start while any are pending. `crates/server/build.rs` tells Cargo to rebuild when anything in `migrations/` changes, so a new file is always picked up. The `.sqlx/` folder holds the offline query data for `query!` (`cargo sqlx prepare --workspace`) and is committed.

## Modules (inside `server`)

| Module       | Owns (Postgres schema)          | Responsibility |
|--------------|---------------------------------|----------------|
| `identity`   | `identity.*`                    | users, login identities, sessions, tokens |
| `org`        | `org.*`                         | terms, groups (cohorts), institution settings |
| `teaching`   | `teaching.*`                    | courses, memberships, publishing; group links and course subgroups are planned |
| `content`    | `content.*`                     | modules, items (lessons and `text` assignments built; other assignment types planned). Planned kinds and types: ADR-0023, ADR-0024. |
| `assessment` | `assessment.*`                  | submissions, attempts, lateness, auto-grading of quizzes |
| `gradebook`  | `gradebook.*`                   | grades (draft/published), grade history, publishing, gradebook view |
| `files`      | `files.*`                       | uploads, storage backends |
| `events`     | `events.*`                      | audit log (built); outbox, webhooks, scheduled jobs (planned) |
| `policy`     | — (no tables)                   | `authorize(&Actor, Action)` — the only place permissions are checked |

Rules ([ADR-0001](docs/decisions/0001-modular-monolith.md)):

1. A module **writes only its own tables**. Other modules change its data through its public Rust functions.
2. **Cross-module reads with JOIN are allowed** for read models such as the gradebook.
3. Inside a module the layers are `routes` (HTTP) → `service` (business rules + `policy`) → `repo` (SQL). HTTP types do not leak into `service`.
4. Every permission check goes through `policy`. There are no ad-hoc role checks in handlers.

### Module layout ([ADR-0016](docs/decisions/0016-module-layout-and-transactions.md))

```
crates/server/src/
  lib.rs    main.rs   app.rs   error.rs   actor.rs   settings.rs   health.rs
  <module>/
    mod.rs      # the only public surface: pub fn router(), pub service functions, pub types
    model.rs    # domain types (public)
    routes.rs   # private (a folder `routes/` once it grows, as in `identity`)
    service.rs  # private
    repo.rs     # private
```

`server` is a library plus a thin binary `learnly-server`. `lib.rs` only declares modules, `main.rs` is the CLI entry, and integration tests use the library.

Private files cannot be reached from other modules, so the compiler enforces module boundaries.

### Transactions ([ADR-0016](docs/decisions/0016-module-layout-and-transactions.md))

- Services and repos take `&mut PgConnection` and never `begin` or `commit`.
- **Writes:** the handler opens a transaction, calls exactly one *writing* service function, and commits. The owning module's service orchestrates other modules and passes the same connection on. Reads and slow preparation (loading a user, verifying and hashing passwords) are separate service calls that run before `begin`, never inside the transaction.
- **Reads:** a pooled connection without a transaction (read-only transaction only for consistent multi-query snapshots).
- **Nothing slow inside a transaction:** no password hashing, no HTTP calls. Files are stored before their metadata is inserted.

## Data model

```
identity
  users            id, email (unique, stored lowercase), display_name,
                   role: admin | teacher | student, status: active | disabled,
                   email_verified_at?, must_change_password, created_at, updated_at
  identities       id, user_id, provider: password | google, subject, password_hash?
                   unique(provider, subject); a password identity has subject = user id
  sessions         id, user_id, token_hash (SHA-256, unique), kind: web | app | cli | pat, name?,
                   created_at, last_seen_at, expires_at       -- revoking = deleting the row
  device_authorizations  id, device_code_hash, user_code, user_id?,
                   status: pending | approved | denied, expires_at    -- CLI login, RFC 8628 (planned)
  email_tokens     -- email verification and password reset (planned, task 004)

org
  terms            id, name, starts_on, ends_on
  groups           id, name                            -- cohorts: class 7-A, group PZ-31
  group_members    group_id, user_id
  subgroups        id, group_id, name                  -- 7-A English 1/2; one level only
  subgroup_members subgroup_id, user_id                -- must also be a member of the group

teaching
  courses          id, kind: regular | template, title, term_id?,
                   status: draft | published | archived, owner_id, created_by
                   -- built: term_id has no foreign key until org.terms exists
  course_groups    course_id, group_id, subgroup_id?   -- live link; no subgroup = whole group
  memberships      course_id, user_id, role: teacher | assistant | student
  effective_members (VIEW) = memberships ∪ students of linked groups / subgroups
  course_subgroups        id, course_id, name           -- lab group 1/2; course-local, flat
  course_subgroup_members course_subgroup_id, user_id

content
  modules          id, course_id, parent_id?, position, title, status: draft | published, publish_at?
                   -- one level of sub-modules: parent_id is null or a top-level module of the same course
  items            id, module_id, course_id, position, kind: lesson | assignment, title,
                   body (jsonb object), status: draft | published, publish_at?
                   -- built: both kinds, lesson and assignment (ADR-0020, ADR-0021)
                   -- planned (ADR-0024): the built lesson is renamed text; link, file and a lesson made of steps (ADR-0026) are added
  assignments      item_id (PK), type (text), config (jsonb), max_points numeric(6,2), due_at?,
                   accept_late (bool), late_until?, on_deadline: keep_draft | auto_submit,
                   max_attempts?                       -- null = unlimited
                   -- built: item_id, type, config, max_points, is_bonus (task 007); due_at, accept_late, late_until, on_deadline and max_attempts arrive with the submissions task (ADR-0011, ADR-0021)

assessment
  submissions      id, assignment_id, student_id, attempt, status: draft | submitted,
                   content (jsonb), submitted_at?, is_late
                   -- at most one draft per (assignment, student); submitted rows are immutable

gradebook
  grades           assignment_id, student_id                      unique together
                   draft_score, draft_feedback, draft_updated_by, draft_updated_at,
                   published_score, published_feedback, published_by, published_at
  grade_history    append-only log of every draft save and publish

files
  files            id, owner_id, storage_key, mime, size, created_at

events
  outbox                 id, type, payload (jsonb), occurred_at, processed_at?
  webhook_subscriptions  id, url, secret, event_types[], active
  webhook_deliveries     event_id, subscription_id, attempts, next_attempt_at, status, last_error
  audit_log              id, actor_id, action, resource_type, resource_id, at, meta (jsonb)
```

Key points:

- **One `courses` table for courses and templates** ([ADR-0005](docs/decisions/0005-courses-and-templates.md)). Creating from a template or saving as a template is a deep copy. Content is never shared live.
- **`effective_members` is a view, not a sync job** ([ADR-0006](docs/decisions/0006-groups-and-membership.md)). A student who leaves a group disappears from the course automatically, and their grades stay because grades are keyed by `user_id`.
- **Every content row carries `course_id`** ([ADR-0020](docs/decisions/0020-content-tree.md)), kept consistent with its parent by composite foreign keys, so a content row reaches its course without joins. Positions are dense integers per sibling sequence (the top-level modules of a course, the sub-modules of a module, the items of a module), unique under deferred constraints; a reorder sends the whole list.
- **The gradebook is a separate module, not a separate tree** ([ADR-0015](docs/decisions/0015-gradebook-module.md), [ADR-0009](docs/decisions/0009-grading-and-gradebook.md)). Every gradebook column is an assignment in the course tree. "+ column" in the gradebook creates an assignment of type `no_submission` in a chosen module. Column order = course tree order.

## Identity and sessions

([ADR-0003](docs/decisions/0003-identity-and-sessions.md), [ADR-0017](docs/decisions/0017-accounts-passwords-sessions.md))

- **Accounts** exist only when an admin creates them (`POST /admin/users`, `learnly-server create-user` / `create-admin`). The system generates a temporary password, shown once, and sets `must_change_password`. Self-registration, email verification and Google arrive in later tasks.
- **Login** `POST /auth/login { email, password, client: web | app }`. `web` sets the `learnly_session` cookie (HttpOnly, SameSite=Lax, `Secure` from `auth.cookie_secure`); `app` returns the token for `Authorization: Bearer`. A wrong email, a wrong password and a disabled account all give the same 401 and take the same time (a dummy hash is verified for unknown emails).
- **Tokens** are 32 random bytes in base64url; only their SHA-256 is stored. Passwords use Argon2id (19 MiB, 2 iterations, 1 lane) on a blocking thread, always before a transaction opens.
- **Lifetimes:** `web` 30 days and `app` 90 days, both sliding; `cli` 90 days and `pat` up to 365 days, absolute. `last_seen_at` and the expiry are written at most once an hour per session.
- **Extractors:** `Actor` (valid session, active user, no password change pending) rejects with 401 or 403. `PendingActor` also accepts `must_change_password = true` and is used only by logout, `GET /me` and change password. The role is read from the database on every request, so a role change or a disabled account applies at once.
- **CSRF:** a cookie-authenticated request that is not GET/HEAD/OPTIONS must carry `Origin` equal to `server.public_origin`, and so must a `web` login. Bearer requests are exempt.
- **Rate limits** on login: per IP (`tower_governor`, key from `server.client_ip_header` when set, else the TCP peer) and per account (in memory, 10 failures per 15 minutes, reset on success). Wrong current passwords on change-password count against the same per-account limit.
- **Admin rules:** disabling a user deletes all their sessions in the same transaction; a password reset issues a new temporary password and deletes all sessions; the last active admin can be neither demoted nor disabled (409, serialized with a row lock). Changing one's own password revokes the user's other sessions.

## API conventions

([ADR-0012](docs/decisions/0012-api-for-multiple-clients.md), [ADR-0018](docs/decisions/0018-api-errors-and-pagination.md))

- Everything is under `/api/v1`. OpenAPI is served at `/api/v1/openapi.json` and `/api/v1/docs`, with `cookie` and `bearer` security schemes.
- Errors are RFC 9457 Problem Details, including framework-generated ones. A request that cannot be accepted is `422` with `errors: [{ field, message }]`; the password policy's feedback comes back this way.
- Lists take `?cursor=&limit=` (default 50, max 100) and return `{ items, next_cursor }`. The cursor is opaque.
- Responses that carry a credential (login token, temporary password) send `Cache-Control: no-store`.

## Configuration

`learnly.toml`, any key overridable with `LEARNLY__SECTION__KEY` (see `learnly.example.toml`):
`server.bind_addr`, `server.public_origin`, `server.client_ip_header?`, `database.url`, `database.max_connections`, `log.format`, `institution.name`, and an optional `[auth]` section (`password_min_length` = 12, `cookie_secure` = true, `allowed_domains` = [], used from task 004). `institution.mode` (school | university) is planned. `institution.grading_scale` (`twelve` | `hundred`) limits assignment points, see [ADR-0022](docs/decisions/0022-grading-scale-setting.md); optional, default `hundred` until `institution.mode` exists, then it follows the mode. Planned (ADR-0025): `editor.html` (default `false`), `editor.video_hosts` and `editor.profiles.<name>.features`, which narrow what a place of the editor allows.

## Authorization

`policy::authorize(&Actor, Action)` decides every check. The resource an action needs travels inside its `Action` variant, so course-level actions can be added without changing existing callers. Today: `ManageOwnAccount` (any role), `ManageUsers` (admin only), the course actions `CreateCourse`, `ViewOwnCourses`, `ViewCourse`, `PublishCourse`, `ManageMembership`, `DeleteCourse` and `TransferOwnership`, and the content actions `ManageContent` and `ViewContent`. A course action carries a `CourseAccess` (the actor's course role, whether they own it, and its status), which `teaching` reads from the database; other modules get it through `teaching::course_access`. `ManageContent` covers viewing drafts, editing and publishing content for staff and admins and ignores the course status, so the content of an archived course stays editable until archiving gets its own ADR. `ViewContent` carries the release state of every row on the path to the target and the evaluation time, because `policy` has no clock. The delete and transfer actions have no endpoint yet.

There are two levels of roles ([ADR-0004](docs/decisions/0004-roles-and-policy.md)).

**Institution level** (`users.role`):

| Action                                         | student | teacher | admin |
|------------------------------------------------|:-------:|:-------:|:-----:|
| Create a course                                |    –    |    ✓    |   ✓   |
| Manage users, roles, groups, terms             |    –    |    –    |   ✓   |

**Course level** (`memberships.role`; the owner is `courses.owner_id`):

| Action                                                      | student | assistant | teacher | owner | admin |
|-------------------------------------------------------------|:-------:|:---------:|:-------:|:-----:|:-----:|
| View published content, own published grades               |    ✓    |     ✓     |    ✓    |   ✓   |   ✓   |
| Submit work (own)                                           |    ✓    |     –     |    –    |   –   |   –   |
| View drafts, edit content, publish content                  |    –    |     ✓     |    ✓    |   ✓   |   ✓   |
| Grade, publish grades                                       |    –    |     ✓     |    ✓    |   ✓   |   ✓   |
| Enroll/remove students, link groups, manage course subgroups|    –    |     ✓     |    ✓    |   ✓   |   ✓   |
| Publish the course ([ADR-0019](docs/decisions/0019-course-publishing.md)) |    –    |     –     |    ✓    |   ✓   |   ✓   |
| Add/remove teachers and assistants                          |    –    |     –     |    ✓    |   ✓   |   ✓   |
| Archive course                                              |    –    |     –     |    ✓    |   ✓   |   ✓   |
| Delete course, transfer ownership                           |    –    |     –     |    –    |   ✓   |   ✓   |

Invariants:

- Course role `teacher` requires institution role `teacher` or `admin`. An institution `student` can be a course `assistant`.
- An institution `teacher` may join any course as `student`.
- An `admin` has no membership rows and acts as an implicit owner everywhere. **Every admin action inside a course is written to `audit_log`** (course creation, publishing, adding and removing members; reads are not audited).
- The owner is also a `teacher` member and can be neither removed nor given another role. A course an admin creates has no owner membership.
- Putting the role a member already has changes nothing. Member writes lock the course row first, so concurrent writers see each other's committed roles.
- Course endpoints answer 404 to anyone who may not see the course, 403 to those who may see it but not do the action.
- The owner is always a course teacher or an admin.

## Visibility rules

- A student sees a course only if it is `published`; a draft or archived course is a 404 for them, in `GET /courses/{id}`, its members and `GET /me/courses`. They can still be enrolled in a draft. Staff and admins see every status.
- A student sees an item only if **the course, every module on the path to it (the module, and its parent if it is a sub-module) and the item** are all `published` and each `publish_at` is null or `<= now()`. List queries filter in SQL, and `policy` checks every row again. Staff and admins see drafts and scheduled rows. A student gets 404, never 403, for anything they may not see.
- A student **never receives `draft_*` grade fields.** Student-facing repo functions do not select those columns, and a test enforces this.
- A teacher sees only `submitted` submissions, never a student's draft.

## Key flows

**Publishing grades** ([ADR-0009](docs/decisions/0009-grading-and-gradebook.md))
Grading writes only `draft_*`. "Publish" runs in one transaction: for every row where draft ≠ published, copy `draft_*` → `published_*`, append to `grade_history`, and emit one `grade.published` event per student. Editing an already published grade changes only the draft. The student keeps seeing the old grade until the next publish.

**Publishing a module** ([ADR-0010](docs/decisions/0010-content-publishing.md))
`POST /modules/{id}/publish { "cascade": true | false }`. The parameter is required and has no default. The web UI asks in a dialog; the CLI and mobile app must choose explicitly. `cascade: true` also publishes the module's sub-modules and every draft item below them, in one transaction. Publishing is idempotent and never changes `publish_at`.

The content endpoints (tag `content`, all under `/api/v1`):

| Endpoint | |
|---|---|
| `POST /courses/{id}/modules` | create a module, or a sub-module with `parent_id` |
| `GET /courses/{id}/content` | the whole tree, not paged (it is bounded and ordered by position, not by id) |
| `PATCH /modules/{id}`, `DELETE /modules/{id}` | edit (`publish_at: null` clears it), delete with everything below |
| `PUT /courses/{id}/modules/order` | reorder the top-level modules, or the sub-modules of `parent_id` |
| `POST /modules/{id}/publish` | publish, `cascade` required |
| `POST /modules/{id}/items`, `PUT /modules/{id}/items/order` | create a lesson or an assignment, reorder a module's items |
| `GET`, `PATCH`, `DELETE /items/{id}`, `POST /items/{id}/publish` | read (with `body`), edit, delete, publish a lesson or an assignment |

Two creates that race for the same last position conflict at commit: the loser gets 409 and retries.

**Submission lifecycle** ([ADR-0011](docs/decisions/0011-submissions.md))
`draft` (autosaved, invisible to staff) → `submitted` (immutable). Resubmitting creates a new attempt, limited by `max_attempts`. At the deadline the draft is kept or auto-submitted according to `on_deadline`. A late submission is allowed only if `accept_late` is set and `now() <= late_until` (if set), and it is marked `is_late`. Staff grade the latest submitted attempt.

**Events** ([ADR-0007](docs/decisions/0007-events-outbox.md))
A state change and its event are written in the same transaction. A worker (a tokio task in the same binary) delivers events to internal handlers (email) and to webhook subscriptions (HMAC-signed, with retries). Delivery is **at-least-once**, so consumers deduplicate by event id. The payload contains ids only. **Draft saves emit no events.**

**Scheduled jobs** (same worker): items reaching `publish_at` → `item.published`; assignments reaching `due_at` with `on_deadline = auto_submit` → submit drafts; outbox delivery; expired-session cleanup.

## Institution modes

`school` or `university`, set in the config file. The mode changes **terminology** (course/subject, module/topic), **the default grading scale** ([ADR-0022](docs/decisions/0022-grading-scale-setting.md)) and **dashboard layout**. It does not change the schema.

Examples: in a school, the template "Algebra 7" becomes courses "Algebra 7-A" and "Algebra 7-B", each linked to its class group. In a university, the template "Databases" becomes "Databases, Fall 2026", linked to one or more academic groups.

## Open questions

- Frontend technology (React SPA vs Leptos). The frontend is a separate app either way ([ADR-0013](docs/decisions/0013-separate-frontend.md)).
- Final grades: computed vs overridden, and where they live.
- Guardians: an institution role linked to specific students.
- Extension model beyond webhooks: assignment-type registry, WASM, LTI 1.3.
