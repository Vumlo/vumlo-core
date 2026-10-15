# Learnly — Architecture

Status: **design, no code yet.**
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
user CLI       ──bearer──┘
admin CLI (runs on the server, same binary)  ────────────────>  PostgreSQL
```

- **One API for every client.** No client-specific endpoints ([ADR-0012](docs/decisions/0012-api-for-multiple-clients.md)).
- **Admin CLI** = subcommands of the server binary: `serve`, `migrate`, `create-admin`, `import-users`. It talks to the database directly.
- **User CLI** = a normal API client with a token. It never touches the database.

## Repository layout

```
crates/
  server/      # HTTP API, modules, worker, admin CLI subcommands
  api-types/   # request/response types shared with the Rust CLI; OpenAPI source
  cli/         # user CLI (API client)
migrations/
docs/decisions/
```

## Modules (inside `server`)

| Module       | Owns (Postgres schema)          | Responsibility |
|--------------|---------------------------------|----------------|
| `identity`   | `identity.*`                    | users, login identities, sessions, tokens |
| `org`        | `org.*`                         | terms, groups (cohorts), grading scales, institution settings |
| `teaching`   | `teaching.*`                    | courses, memberships, group links, course subgroups |
| `content`    | `content.*`                     | modules, items, assignments and assignment types |
| `assessment` | `assessment.*`                  | submissions, attempts, lateness, auto-grading of forms |
| `gradebook`  | `gradebook.*`                   | grades (draft/published), grade history, publishing, gradebook view |
| `files`      | `files.*`                       | uploads, storage backends |
| `events`     | `events.*`                      | outbox, webhooks, audit log, scheduled jobs |
| `policy`     | — (no tables)                   | `authorize(actor, action, resource)` — the only place permissions are checked |

Rules ([ADR-0001](docs/decisions/0001-modular-monolith.md)):

1. A module **writes only its own tables**. Other modules change its data through its public Rust functions.
2. **Cross-module reads with JOIN are allowed** for read models such as the gradebook.
3. Inside a module the layers are `routes` (HTTP) → `service` (business rules + `policy`) → `repo` (SQL). HTTP types do not leak into `service`.
4. Every permission check goes through `policy`. There are no ad-hoc role checks in handlers.

### Module layout ([ADR-0016](docs/decisions/0016-module-layout-and-transactions.md))

```
crates/server/src/
  lib.rs    main.rs   app.rs   error.rs   actor.rs
  <module>/
    mod.rs      # the only public surface: pub fn router(), pub service functions, pub types
    model.rs    # domain types (public)
    routes.rs   # private
    service.rs  # private
    repo.rs     # private
```

`server` is a library plus a thin binary `learnly-server`. `lib.rs` only declares modules, `main.rs` is the CLI entry, and integration tests use the library.

Private files cannot be reached from other modules, so the compiler enforces module boundaries.

### Transactions ([ADR-0016](docs/decisions/0016-module-layout-and-transactions.md))

- Services and repos take `&mut PgConnection` and never `begin` or `commit`.
- **Writes:** the handler opens a transaction, calls exactly one service function, and commits. The owning module's service orchestrates other modules and passes the same connection on.
- **Reads:** a pooled connection without a transaction (read-only transaction only for consistent multi-query snapshots).
- **Nothing slow inside a transaction:** no password hashing, no HTTP calls. Files are stored before their metadata is inserted.

## Data model

```
identity
  users            id, email (unique), display_name,
                   role: admin | teacher | student, status: active | disabled
  identities       id, user_id, provider: password | google, subject, password_hash?
                   unique(provider, subject)
  sessions         id, user_id, token_hash, kind: web | app | cli | pat, name?,
                   created_at, last_seen_at, expires_at
  device_authorizations  id, device_code_hash, user_code, user_id?,
                   status: pending | approved | denied, expires_at    -- CLI login, RFC 8628

org
  terms            id, name, starts_on, ends_on
  groups           id, name                            -- cohorts: class 7-A, group PZ-31
  group_members    group_id, user_id
  subgroups        id, group_id, name                  -- 7-A English 1/2; one level only
  subgroup_members subgroup_id, user_id                -- must also be a member of the group
  grading_scales   id, name, kind, config (jsonb)      -- 12-point, 100-point + ECTS, ...

teaching
  courses          id, kind: regular | template, title, term_id?, grading_scale_id,
                   status: draft | published | archived, owner_id, created_by
  course_groups    course_id, group_id, subgroup_id?   -- live link; no subgroup = whole group
  memberships      course_id, user_id, role: teacher | assistant | student
  effective_members (VIEW) = memberships ∪ students of linked groups / subgroups
  course_subgroups        id, course_id, name           -- lab group 1/2; course-local, flat
  course_subgroup_members course_subgroup_id, user_id

content
  modules          id, course_id, position, title, status: draft | published, publish_at?
  items            id, module_id, position, kind: lesson | assignment, title,
                   body (jsonb), status: draft | published, publish_at?
  assignments      item_id (PK), type (text), config (jsonb), max_points, due_at?,
                   accept_late (bool), late_until?, on_deadline: keep_draft | auto_submit,
                   max_attempts?                       -- null = unlimited

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
- **The gradebook is a separate module, not a separate tree** ([ADR-0015](docs/decisions/0015-gradebook-module.md), [ADR-0009](docs/decisions/0009-grading-and-gradebook.md)). Every gradebook column is an assignment in the course tree. "+ column" in the gradebook creates an assignment of type `no_submission` in a chosen module. Column order = course tree order.

## Authorization

There are two levels of roles ([ADR-0004](docs/decisions/0004-roles-and-policy.md)).

**Institution level** (`users.role`):

| Action                                         | student | teacher | admin |
|------------------------------------------------|:-------:|:-------:|:-----:|
| Create a course                                |    –    |    ✓    |   ✓   |
| Manage users, roles, groups, terms, scales     |    –    |    –    |   ✓   |

**Course level** (`memberships.role`; the owner is `courses.owner_id`):

| Action                                                      | student | assistant | teacher | owner | admin |
|-------------------------------------------------------------|:-------:|:---------:|:-------:|:-----:|:-----:|
| View published content, own published grades               |    ✓    |     ✓     |    ✓    |   ✓   |   ✓   |
| Submit work (own)                                           |    ✓    |     –     |    –    |   –   |   –   |
| View drafts, edit content, publish content                  |    –    |     ✓     |    ✓    |   ✓   |   ✓   |
| Grade, publish grades                                       |    –    |     ✓     |    ✓    |   ✓   |   ✓   |
| Enroll/remove students, link groups, manage course subgroups|    –    |     ✓     |    ✓    |   ✓   |   ✓   |
| Add/remove teachers and assistants                          |    –    |     –     |    ✓    |   ✓   |   ✓   |
| Change grading scale, archive course                        |    –    |     –     |    ✓    |   ✓   |   ✓   |
| Delete course, transfer ownership                           |    –    |     –     |    –    |   ✓   |   ✓   |

Invariants:

- Course role `teacher` requires institution role `teacher` or `admin`. An institution `student` can be a course `assistant`.
- An institution `teacher` may join any course as `student`.
- An `admin` has no membership rows and acts as an implicit owner everywhere. **Every admin action inside a course is written to `audit_log`.**
- The owner is always a course teacher or an admin.

## Visibility rules

- A student sees an item only if **the course, the module and the item** are all `published` and `publish_at <= now()`.
- A student **never receives `draft_*` grade fields.** Student-facing repo functions do not select those columns, and a test enforces this.
- A teacher sees only `submitted` submissions, never a student's draft.

## Key flows

**Publishing grades** ([ADR-0009](docs/decisions/0009-grading-and-gradebook.md))
Grading writes only `draft_*`. "Publish" runs in one transaction: for every row where draft ≠ published, copy `draft_*` → `published_*`, append to `grade_history`, and emit one `grade.published` event per student. Editing an already published grade changes only the draft. The student keeps seeing the old grade until the next publish.

**Publishing a module** ([ADR-0010](docs/decisions/0010-content-publishing.md))
`POST /modules/{id}/publish { "cascade": true | false }`. The parameter is required and has no default. The web UI asks in a dialog; the CLI and mobile app must choose explicitly.

**Submission lifecycle** ([ADR-0011](docs/decisions/0011-submissions.md))
`draft` (autosaved, invisible to staff) → `submitted` (immutable). Resubmitting creates a new attempt, limited by `max_attempts`. At the deadline the draft is kept or auto-submitted according to `on_deadline`. A late submission is allowed only if `accept_late` is set and `now() <= late_until` (if set), and it is marked `is_late`. Staff grade the latest submitted attempt.

**Events** ([ADR-0007](docs/decisions/0007-events-outbox.md))
A state change and its event are written in the same transaction. A worker (a tokio task in the same binary) delivers events to internal handlers (email) and to webhook subscriptions (HMAC-signed, with retries). Delivery is **at-least-once**, so consumers deduplicate by event id. The payload contains ids only. **Draft saves emit no events.**

**Scheduled jobs** (same worker): items reaching `publish_at` → `item.published`; assignments reaching `due_at` with `on_deadline = auto_submit` → submit drafts; outbox delivery; expired-session cleanup.

## Institution modes

`school` or `university`, set in the config file. The mode changes **terminology** (course/subject, module/topic), **the default grading scale** and **dashboard layout**. It does not change the schema.

Examples: in a school, the template "Algebra 7" becomes courses "Algebra 7-A" and "Algebra 7-B", each linked to its class group. In a university, the template "Databases" becomes "Databases, Fall 2026", linked to one or more academic groups.

## Open questions

- Frontend technology (React SPA vs Leptos). The frontend is a separate app either way ([ADR-0013](docs/decisions/0013-separate-frontend.md)).
- Final grades: computed vs overridden, and where they live.
- Guardians: an institution role linked to specific students.
- Extension model beyond webhooks: assignment-type registry, WASM, LTI 1.3.
