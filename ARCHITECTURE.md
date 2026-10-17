# Learnly — Architecture

Status: **in progress.** Implemented: the foundation (config, migrations, health, OpenAPI, Problem Details), `identity` + `policy` (users, password login, sessions, admin user management), the first slice of `teaching` (courses, members, publishing) with a minimal `events.audit_log`, the `content` tree (modules, sub-modules, `text` items and assignments of type `text` or `no_submission`, whose bodies are block documents), the `gradebook` (draft and published grades, publishing, the staff gradebook, a student's published grades), the first event in `events.outbox`, and `org` with the installation record (the institution mode, ADR-0031) terms, groups and subgroups (ADR-0032), and in `teaching` the group links, the live roster, course exclusions and course editing (ADR-0033). Everything else below is still design.
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
- **`learnly-server`** = the server binary. Subcommands: `serve`, `migrate`, `create-admin`, `create-user` (`import-users` is planned). Admin subcommands talk to the database directly. Every one that uses the database records or checks the institution mode first (see Configuration).
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

**Migrations** are plain `.sql` files in `migrations/`, append-only, embedded into `learnly-server` at compile time with `sqlx::migrate!` and applied by `learnly-server migrate`. `serve` refuses to start while any are pending, and then checks the institution mode against `org.installation` (ADR-0031). `crates/server/build.rs` tells Cargo to rebuild when anything in `migrations/` changes, so a new file is always picked up. The `.sqlx/` folder holds the offline query data for `query!` in the code and in the tests (`cargo sqlx prepare --workspace -- --all-targets`) and is committed; CI also builds every target offline (`SQLX_OFFLINE=true cargo check --workspace --all-targets`).

## Modules (inside `server`)

| Module       | Owns (Postgres schema)          | Responsibility |
|--------------|---------------------------------|----------------|
| `identity`   | `identity.*`                    | users, login identities, sessions, tokens |
| `org`        | `org.*`                         | the installation record (the institution mode, ADR-0031); terms, groups (cohorts) and subgroups with their members (ADR-0032) |
| `teaching`   | `teaching.*`                    | courses, memberships, publishing, group links, exclusions and the roster view (ADR-0033); course subgroups are planned |
| `content`    | `content.*`                     | modules, items (`text` items and assignments of type `text` and `no_submission` built, their bodies are block documents (ADR-0025, ADR-0027); other assignment types planned). Planned kinds and types: ADR-0023, ADR-0024. |
| `assessment` | `assessment.*`                  | submissions, attempts, lateness, auto-grading of quizzes |
| `gradebook`  | `gradebook.*`                   | grades (draft/published), grade history, publishing, gradebook view, my grades |
| `files`      | `files.*`                       | uploads, storage backends |
| `events`     | `events.*`                      | audit log and outbox table with `events::emit` (built); the worker, webhooks and scheduled jobs (planned) |
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
  users            id, email (unique, stored lowercase), name, surname (ADR-0029),
                   role: admin | teacher | student, status: active | disabled,
                   email_verified_at?, must_change_password, created_at, updated_at
  identities       id, user_id, provider: password | google, subject, password_hash?
                   unique(provider, subject); a password identity has subject = user id
  sessions         id, user_id, token_hash (SHA-256, unique), kind: web | app | cli | pat, name?,
                   created_at, last_seen_at, expires_at       -- revoking = deleting the row
  device_authorizations  id, device_code_hash, user_code, user_id?,
                   status: pending | approved | denied, expires_at    -- CLI login, RFC 8628 (planned)
  email_tokens     -- email verification and password reset (planned, task 004)

org                                                -- built: ADR-0031, ADR-0032
  installation     singleton, mode: school | university     -- one row, written by the first start
  terms            id, name (trimmed, 1-100, unique), starts_on, ends_on   -- ends_on >= starts_on; flat
  groups           id, name (trimmed, 1-100, unique), created_at           -- cohorts: class 7-A, group PZ-31
  group_members    group_id, user_id                   -- primary key together; students only (by the service)
  subgroups        id, group_id, name                  -- 7-A English 1/2; one level only; unique(group_id, name)
  subgroup_members subgroup_id, group_id, user_id      -- foreign keys (subgroup_id, group_id) -> subgroups and
                                                       -- (group_id, user_id) -> group_members, both ON DELETE CASCADE:
                                                       -- a subgroup member is a group member, and leaves with them

teaching
  courses          id, kind: regular | template, title, term_id?,
                   status: draft | published | archived, owner_id, created_by
                   -- built: term_id -> org.terms ON DELETE RESTRICT; a term in use cannot be deleted (ADR-0032)
  course_groups    id, course_id, group_id, subgroup_id?, created_by, created_at
                   -- built (ADR-0033): live link; no subgroup = whole group. unique (course_id, group_id, subgroup_id)
                   -- with NULLs equal; group_id -> org.groups and (subgroup_id, group_id) -> org.subgroups,
                   -- both ON DELETE RESTRICT (a linked group or subgroup cannot be deleted); the course deletes its links
  course_exclusions course_id, user_id, created_by, created_at
                   -- built: a person taken off one course although a linked group would put them on it
  memberships      course_id, user_id, role: teacher | assistant | student     -- direct memberships, written by teaching only
  group_roster (VIEW) course_id, user_id                -- students reachable through a link; teaching's own building block
  effective_members (VIEW) course_id, user_id, role, source: direct | group
                   -- built (ADR-0033): one row per (course, user). Direct membership first (its role, source 'direct'),
                   -- else a member of a linked group or subgroup is a 'student' with source 'group', unless excluded
  course_subgroups        id, course_id, name           -- lab group 1/2; course-local, flat
  course_subgroup_members course_subgroup_id, user_id

content
  modules          id, course_id, parent_id?, position, title, status: draft | published, publish_at?
                   -- one level of sub-modules: parent_id is null or a top-level module of the same course
  items            id, module_id, course_id, position, kind: text | assignment, title,
                   body (jsonb object), status: draft | published, publish_at?
                   -- built: both kinds, text and assignment (ADR-0020, ADR-0021, ADR-0024)
                   -- the body of a text item and of an assignment (its description) is a block document (ADR-0025, ADR-0027); the default is the empty document
                   -- planned (ADR-0024): link, file and a lesson made of steps (ADR-0026) are added
  assignments      item_id (PK), type: text | no_submission, config (jsonb), max_points numeric(6,2), due_at?,
                   accept_late (bool), late_until?, on_deadline: keep_draft | auto_submit,
                   max_attempts?                       -- null = unlimited
                   -- built: item_id, type, config, max_points, is_bonus (task 007); due_at, accept_late, late_until, on_deadline and max_attempts arrive with the submissions task (ADR-0011, ADR-0021)

assessment
  submissions      id, assignment_id, student_id, attempt, status: draft | submitted,
                   content (jsonb), submitted_at?, is_late
                   -- at most one draft per (assignment, student); submitted rows are immutable

gradebook                                          -- ADR-0028
  grades           assignment_id, student_id                      primary key together; no foreign key to the assignment (ADR-0021)
                   draft_score numeric(6,2)?, draft_feedback?, draft_updated_by, draft_updated_at,
                   published_score?, published_feedback?, published_by?, published_at?
                   -- score >= 0 by CHECK, score <= max_points by the service; feedback trimmed, 1 to 4000 characters
                   -- a cell is "published" for the student when published_score or published_feedback is set
  grade_history    id, assignment_id, student_id, action: save_draft | publish, actor_id, at,
                   score?, feedback?                   -- the values after the action; append-only (a trigger refuses UPDATE; only the purge deletes)

files
  files            id, owner_id, storage_key, mime, size, created_at

events
  outbox                 id, type, version, payload (jsonb, ids only), occurred_at, processed_at?
                         -- built; the only event so far is grade.published
  webhook_subscriptions  id, url, secret, event_types[], active
  webhook_deliveries     event_id, subscription_id, attempts, next_attempt_at, status, last_error
  audit_log              id, actor_id?, source: api | cli, action, resource_type, resource_id, at, meta (jsonb)
                         -- built (ADR-0034): an api row has an actor, a cli row (create-admin, create-user) has none (a check)
```

Key points:

- **One `courses` table for courses and templates** ([ADR-0005](docs/decisions/0005-courses-and-templates.md)). Creating from a template or saving as a template is a deep copy. Content is never shared live.
- **`effective_members` is a view, not a sync job** ([ADR-0006](docs/decisions/0006-groups-and-membership.md)). A student who leaves a group disappears from the course automatically, and their grades stay because grades are keyed by `user_id`. The view is `teaching`'s declared read interface for who takes part in a course, with the columns `course_id`, `user_id`, `role` and `source`: other modules read it and never `teaching.memberships`, `course_groups`, `course_exclusions` or `group_roster`, and nothing outside `org` reads `org.group_members` or `org.subgroup_members` (`crates/server/tests/architecture.rs` checks this). Precedence is direct membership, then exclusion, then group, so one row per (course, user) holds however many links reach a user. Everything in `teaching` that reads a role (`course_access`, the `ViewCourse` decision, "my courses", the member list, the checks of the membership writes) reads the view; writes of direct memberships go to the table.
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
- **Admin rules** (every one of these writes is audited, ADR-0034): disabling a user deletes all their sessions in the same transaction; a password reset issues a new temporary password and deletes all sessions; the last active admin can be neither demoted nor disabled (409, serialized with a row lock). Changing one's own password revokes the user's other sessions.

## API conventions

([ADR-0012](docs/decisions/0012-api-for-multiple-clients.md), [ADR-0018](docs/decisions/0018-api-errors-and-pagination.md))

- Everything is under `/api/v1`. OpenAPI is served at `/api/v1/openapi.json` and `/api/v1/docs`, with `cookie` and `bearer` security schemes.
- Errors are RFC 9457 Problem Details, including framework-generated ones. A request that cannot be accepted is `422` with `errors: [{ field, message }]`; the password policy's feedback comes back this way.
- Lists take `?cursor=&limit=` (default 50, max 100) and return `{ items, next_cursor }`. The cursor is opaque.
- Responses that carry a credential (login token, temporary password) send `Cache-Control: no-store`.

## Configuration

`learnly.toml`, any key overridable with `LEARNLY__SECTION__KEY` (see `learnly.example.toml`):
`server.bind_addr`, `server.public_origin`, `server.client_ip_header?`, `database.url`, `database.max_connections`, `log.format`, `institution.name`, `institution.mode`, and an optional `[auth]` section (`password_min_length` = 12, `cookie_secure` = true, `allowed_domains` = [], used from task 004). `institution.mode` (`school` | `university`) is **required, has no default and is fixed for the life of the installation** ([ADR-0031](docs/decisions/0031-institution-mode-required-and-fixed.md)): a missing or unknown value stops the server at startup, naming the key and the allowed values. The first start records the mode in `org.installation` (one row); a later start whose config says another mode refuses to start and names both values. The check, `org::record_mode(&mut conn, mode)`, runs in `main.rs` before the server binds (`serve`), after the migrations (`migrate`) and before `create-admin` / `create-user`; it is insert-if-absent, then read, so concurrent first starts cannot store two modes. `institution.grading_scale` (`twelve` | `hundred`) limits assignment points, see [ADR-0022](docs/decisions/0022-grading-scale-setting.md); optional, and when absent it follows the mode (school gives `twelve`, university gives `hundred`). The scale is resolved once, when the `[institution]` section loads (`TryFrom<InstitutionSection> for InstitutionSettings`, which also checks the mode); `institution.grading_scale` in the loaded settings is the scale that applies, and nothing else reads the raw key. `editor.profiles.<name>.features` and `editor.video_providers` (default `["youtube", "vimeo"]`; an empty list switches video off) narrow what a place of the editor allows, see [ADR-0025](docs/decisions/0025-block-documents.md) and [ADR-0027](docs/decisions/0027-document-types-and-video-blocks.md). The configuration is plain data. A configuration can only narrow: `AppState::new` resolves it once, when the server starts, and one that widens a profile or names an unknown profile, feature or provider stops the server there with a message that names the key and the value. Handlers and services receive the resolved profiles from `AppState`. Planned: `editor.html` (default `false`).

## Authorization

`policy::authorize(&Actor, Action)` decides every check. The resource an action needs travels inside its `Action` variant, so course-level actions can be added without changing existing callers. Today: `ManageOwnAccount` (any role), `ManageUsers` (admin only), the course actions `CreateCourse`, `ViewOwnCourses`, `ViewCourse`, `PublishCourse`, `ManageMembership`, `DeleteCourse` and `TransferOwnership`, the content actions `ManageContent` and `ViewContent`, `ViewEditorProfiles` (any role), and the gradebook actions `GradeAssignment`, `PublishGrades` and `ViewGradebook` (staff and admins, any course status) and `ViewOwnGrades` (only a student of a published course; staff and admins read the gradebook instead), `ManageTerms`, `ManageGroups` and `ViewGroups` (admins only) and `ViewTerms` (any role), `ManageRoster` (links, unlinking, the group options, exclusions) and `EditCourse` (title and term), both for assistants, teachers, owners and admins in every course status, with `group_role_allowed` (only an institution `student` is a group member) next to `course_role_allowed` (ADR-0032). A course action carries a `CourseAccess` (the actor's course role, whether they own it, and its status), which `teaching` reads from the database; `content` and `gradebook` get it through `teaching::course_access`, the one read function of another module that `gradebook` may call ([ADR-0030](docs/decisions/0030-gradebook-lock-covers-deletes-and-course-access.md)). `ManageContent` covers viewing drafts, editing and publishing content for staff and admins and ignores the course status, so the content of an archived course stays editable until archiving gets its own ADR. `ViewContent` carries the release state of every row on the path to the target and the evaluation time, because `policy` has no clock. The delete and transfer actions have no endpoint yet.

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
- **Admin writes outside a course are audited too** ([ADR-0034](docs/decisions/0034-admin-writes-outside-a-course-are-audited.md)): in `identity`, `user.create`, `user.rename`, `user.role_change`, `user.disable`, `user.enable` and `user.password_reset` (resource type `user`, the target as the resource), one row for each kind of change, in the same transaction. A refused write, a change that changes nothing, a read and a user's own account actions write nothing. The meta holds ids, roles and field names, never a password, a hash, a token, an email or a name. The server CLI's `create-admin` and `create-user` write `user.create` with no actor and the source `cli`.
- The owner is also a `teacher` member and can be neither removed nor given another role. A course an admin creates has no owner membership.
- Putting the role a member already has on the roster changes nothing, also for a student who comes from a linked group. Member, link and exclusion writes lock the course row first and read roles in a later statement, so concurrent writers see each other's committed roles and links.
- Course endpoints answer 404 to anyone who may not see the course, 403 to those who may see it but not do the action.
- The owner is always a course teacher or an admin.

## Visibility rules

- A student sees a course only if it is `published`; a draft or archived course is a 404 for them, in `GET /courses/{id}`, its members and `GET /me/courses`. They can still be enrolled in a draft. Staff and admins see every status.
- A student sees an item only if **the course, every module on the path to it (the module, and its parent if it is a sub-module) and the item** are all `published` and each `publish_at` is null or `<= now()`. List queries filter in SQL, and `policy` checks every row again. Staff and admins see drafts and scheduled rows. A student gets 404, never 403, for anything they may not see.
- A student **never receives `draft_*` grade fields.** Student-facing repo functions do not select those columns, and a test enforces this.
- A teacher sees only `submitted` submissions, never a student's draft.
- A student sees a `no_submission` assignment like any item: only when the course, module and item are published ([ADR-0028](docs/decisions/0028-gradebook-grades-publishing-read-models.md)). `GET /courses/{id}/my-grades` returns published values only, for assignments visible to the student now.

## Key flows

**Grades and publishing** ([ADR-0009](docs/decisions/0009-grading-and-gradebook.md), [ADR-0028](docs/decisions/0028-gradebook-grades-publishing-read-models.md))
Grading writes only `draft_*`: `PUT /assignments/{id}/grades/{student_id}` replaces the draft cell (`score` a decimal string from 0 to the assignment's `max_points`, or `null`; `feedback` plain text or `null`), appends to `grade_history` and emits nothing. "Publish" (`POST /assignments/{id}/grades/publish`, body `{"students": "all"}` or a list, required) runs in one transaction: for every current student whose draft differs from the published value it copies `draft_*` → `published_*`, appends to `grade_history`, and emits one `grade.published` event per student whose published value became non-empty. Publishing an empty draft over a published grade retracts it and emits no event. Editing a published grade changes only the draft; the student keeps seeing the old grade until the next publish. A grade write and a change of `max_points` or a delete on the same assignment are serialized through the assignment row in `content.assignments` (the grade write holds it `FOR SHARE`, the others `FOR UPDATE`, [ADR-0030](docs/decisions/0030-gradebook-lock-covers-deletes-and-course-access.md)).

Lowering `max_points` below a score already given, and deleting an assignment, a module or a course that holds a grade with a value, are 409; rows with no value are purged with their history by the delete. There is no course delete yet: it must call `gradebook::has_grades_in_course` and `purge_course`.

The gradebook endpoints (tag `gradebook`):

| Endpoint | |
|---|---|
| `PUT /assignments/{id}/grades/{student_id}` | staff and admins: save a draft cell; 404 when the student is not a current student of the course; 422 on `score` / `feedback` |
| `POST /assignments/{id}/grades/publish` | staff and admins: publish `all` or a listed set of students; returns the students whose published value was replaced |
| `GET /courses/{id}/gradebook` | staff and admins: columns (every assignment in tree order, drafts included) and the current students as rows by surname, name and id, paged by student, with both slots and `has_unpublished_changes` per cell |
| `GET /courses/{id}/my-grades` | students only: published values for assignments visible now, no draft field |

A cell exists only where a grade row exists; every cell carries its `assignment_id`, and a missing cell means "no grade". The roster is the students in the view `teaching.effective_members`, direct and through linked groups.

**Publishing a module** ([ADR-0010](docs/decisions/0010-content-publishing.md))
`POST /modules/{id}/publish { "cascade": true | false }`. The parameter is required and has no default. The web UI asks in a dialog; the CLI and mobile app must choose explicitly. `cascade: true` also publishes the module's sub-modules and every draft item below them, in one transaction. Publishing is idempotent and never changes `publish_at`.

The content endpoints (tag `content`, all under `/api/v1`):

| Endpoint | |
|---|---|
| `POST /courses/{id}/modules` | create a module, or a sub-module with `parent_id` |
| `GET /courses/{id}/content` | the whole tree, not paged (it is bounded and ordered by position, not by id) |
| `PATCH /modules/{id}`, `DELETE /modules/{id}` | edit (`publish_at: null` clears it), delete with everything below (409 when an assignment below has a grade) |
| `PUT /courses/{id}/modules/order` | reorder the top-level modules, or the sub-modules of `parent_id` |
| `POST /modules/{id}/publish` | publish, `cascade` required |
| `POST /modules/{id}/items`, `PUT /modules/{id}/items/order` | create a `text` item or an assignment, reorder a module's items; a `body` is validated as a document only after the caller is authorized (404, then 403, then 422) |
| `GET`, `PATCH`, `DELETE /items/{id}`, `POST /items/{id}/publish` | read (with `body`), edit, delete, publish a `text` item or an assignment; lowering `max_points` below a given score and deleting a graded assignment are 409 |
| `GET /editor/profiles` | any signed-in user: the document profiles as enforced (features, single block or not, limits) and the enabled video providers |

Two creates that race for the same last position conflict at commit: the loser gets 409 and retries.

**Terms, groups and subgroups** ([ADR-0032](docs/decisions/0032-terms-groups-and-subgroups.md))
Admins create, rename, change and delete them; every one of those writes is audited (resource types `term`, `group`, `subgroup`) and none emits an event. Any signed-in user reads terms; groups, subgroups and their members are read by admins only for now. The tags are `terms` and `groups` (subgroups are under `groups`).

| Endpoint | |
|---|---|
| `POST /terms`, `GET /terms`, `GET /terms/{id}`, `PATCH /terms/{id}`, `DELETE /terms/{id}` | `ends_on` before `starts_on` is 422 (checked after merging a `PATCH`); a duplicate name is 409; deleting a term that a course uses is 409 (the foreign key) |
| `POST /groups`, `GET /groups`, `GET /groups/{id}`, `PATCH /groups/{id}` (rename), `DELETE /groups/{id}` | a duplicate name is 409; the delete takes the subgroups and members with it |
| `GET /groups/{id}/members` | by surname, name and id; the cursor is a user id |
| `POST /groups/{id}/members` | `{user_ids}`, 1 to 1000; all or nothing and idempotent (`added` lists the new members); an unknown user or a user who is not a student is 422 on `user_ids[i]` |
| `DELETE /groups/{id}/members/{user_id}` | also removes the user from the group's subgroups |
| `POST /groups/{id}/subgroups`, `GET /groups/{id}/subgroups` | a name is unique inside the group |
| `PATCH /subgroups/{id}` (rename), `DELETE /subgroups/{id}` | |
| `GET /subgroups/{id}/members`, `POST /subgroups/{id}/members`, `DELETE /subgroups/{id}/members/{user_id}` | an entry that is not a member of the group is 422 on `user_ids[i]` |

Every write below a group takes the group row `FOR UPDATE` first and reads memberships in a later statement, so a removal from the group racing with an add to its subgroup ends consistent; the composite foreign key is the backstop. A role change of a user does not remove their memberships. Unknown ids in the path are 404, in the body 422. `POST /courses` takes an optional `term_id`; a term that does not exist is 422 on `term_id`.

**Group links, the live roster and exclusions** ([ADR-0033](docs/decisions/0033-group-links-effective-roster-and-exclusions.md))
A course links whole groups or single subgroups; their students are students of the course while the link exists. A teacher can take one person off one course without touching the group. Assistants, teachers, owners and admins do all of it (the matrix row "Enroll/remove students, link groups" is unchanged); admin writes are audited (`course.group.link`, `course.group.unlink`, `course.member.remove` with `excluded`, `course.exclusion.restore`, `course.update`). Tag `courses`.

| Endpoint | |
|---|---|
| `GET /courses/{id}/groups`, `POST /courses/{id}/groups`, `DELETE /courses/{id}/groups/{link_id}` | the links, in link-id order; `POST {group_id, subgroup_id?}` is `201`, or `200` with the existing link; an unknown group is `422` on `group_id`, an unknown subgroup or one of another group `422` on `subgroup_id` |
| `GET /courses/{id}/group-options` | groups with their subgroups, ids and names only, by group id; the only group read teachers and assistants have (`org`'s own endpoints stay admin only) |
| `GET /courses/{id}/exclusions`, `DELETE /courses/{id}/exclusions/{user_id}` | list (by user id) and restore |
| `GET /courses/{id}/members` | the roster, each entry with `source`; `PUT` clears the user's exclusion when it writes a direct row |
| `DELETE /courses/{id}/members/{user_id}` | a group student, or a direct student whom a linked group would still put on the roster, is excluded; a direct teacher or assistant is only deleted; `404` when the user is not on the roster |
| `PATCH /courses/{id}` | `{title?, term_id?}`: an absent field changes nothing, `term_id: null` clears; a student is `403`; a template has no term (`422` on `term_id`), nor does an unknown term |

Deleting a group or a subgroup that a course uses is `409` (the foreign keys). An exclusion never touches grades or submissions.

**Submission lifecycle** ([ADR-0011](docs/decisions/0011-submissions.md))
`draft` (autosaved, invisible to staff) → `submitted` (immutable). Resubmitting creates a new attempt, limited by `max_attempts`. At the deadline the draft is kept or auto-submitted according to `on_deadline`. A late submission is allowed only if `accept_late` is set and `now() <= late_until` (if set), and it is marked `is_late`. Staff grade the latest submitted attempt.

**Events** ([ADR-0007](docs/decisions/0007-events-outbox.md))
A state change and its event are written in the same transaction. A worker (a tokio task in the same binary) delivers events to internal handlers (email) and to webhook subscriptions (HMAC-signed, with retries). Delivery is **at-least-once**, so consumers deduplicate by event id. The payload contains ids only. **Draft saves emit no events.**

**Scheduled jobs** (same worker): items reaching `publish_at` → `item.published`; assignments reaching `due_at` with `on_deadline = auto_submit` → submit drafts; outbox delivery; expired-session cleanup.

## Institution modes

`school` or `university`, set in the config file before the first start and fixed afterwards ([ADR-0031](docs/decisions/0031-institution-mode-required-and-fixed.md)). Today it controls only **the default grading scale** ([ADR-0022](docs/decisions/0022-grading-scale-setting.md)). It is meant to control **terminology** (course/subject, module/topic), **dashboard layout** and the **final-grade model** later; none of that is decided. The direction for final grades: the teacher enters the final grade (a semester or year grade at a school, the course grade at a university), and the system computes and suggests a value, from the marks at a school and from the sum of points at a university. The mode does not change tables; it selects defaults, limits and, later, the computation of final grades.

Examples: in a school, the template "Algebra 7" becomes courses "Algebra 7-A" and "Algebra 7-B", each linked to its class group. In a university, the template "Databases" becomes "Databases, Fall 2026", linked to one or more academic groups.

## Known limits

- **Rights are checked before the row lock.** `gradebook` and `content` authorize first and lock the assignment row after, so a role taken away in the same instant can still write one grade. There is no course-level lock.
- **Disabled students stay in the gradebook.** A disabled user who is a student of a course is still listed and graded; the user status is not filtered.
- **The roster is `teaching.effective_members`, live.** The gradebook and `teaching` read the students of a course from this one view; the students of linked groups and subgroups are on it without a sync job. Changes to a group's members show at once on every linked course.
- **An exclusion can go stale.** It stays until restored. A student who left the group since is excluded from nothing; if they are added to the group again they are still excluded.
- **Removing an assistant or a teacher who is in a linked group leaves them a student of the course.** Only a student's removal records an exclusion; remove the person from the group, or exclude them after the removal, to take them off.
- **A group student cannot be pinned.** Putting a student who comes from a linked group as `student` changes nothing (they already hold that role), so no direct membership is written; a direct membership written before the link is the one that survives a later unlink.
- **The yearly rollover is manual.** Rename the group and change its members; the linked courses follow (ADR-0032, ADR-0033).
- **There is no course deletion yet.** When it is added it must lock the assignment rows of the course `FOR UPDATE`, then call `gradebook::has_grades_in_course` (409 if true) and `purge_course`, in one transaction. `teaching` then calls `gradebook` while `gradebook` calls `teaching`: a module-level cycle ([ADR-0030](docs/decisions/0030-gradebook-lock-covers-deletes-and-course-access.md)).

- **The rollover of school classes is manual.** There is no copy-to-next-year action: rename the group and change its members (ADR-0032).
- **Group reads are admin only.** Teachers and assistants read groups through the course-scoped group options (ids and names, no members); `org`'s own endpoints and the member lists stay admin only.
- **A role change keeps the memberships.** A user who stops being a student stays in their groups until an admin removes them.
- **The member lists read `identity.users`** with a read-only JOIN to sort by surname and name, as the gradebook roster does.
- **A term in use stays undeletable** until its courses move to another term (`PATCH /courses/{id}`) or are gone.

## Open questions

- Frontend technology (React SPA vs Leptos). The frontend is a separate app either way ([ADR-0013](docs/decisions/0013-separate-frontend.md)).
- Final grades: where they live, periods, computation and rounding. The direction (teacher-entered, system-suggested) is recorded in [ADR-0031](docs/decisions/0031-institution-mode-required-and-fixed.md).
- Guardians: an institution role linked to specific students.
- Extension model beyond webhooks: assignment-type registry, WASM, LTI 1.3.
