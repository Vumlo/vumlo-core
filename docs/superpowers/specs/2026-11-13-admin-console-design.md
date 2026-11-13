# Admin console: a separate door for administration

Status: draft for approval. Date: 2026-11-13.

## 1. Intent

The person who runs a Vumlo installation is often not a developer. Today they have two bad choices: a CLI with
many commands (`vumlo-cli`), or one `/admin` area inside the student web app, behind the same login and cookie
as every student. The owner wants:

- two things to learn, not three: **create-vumlo** (install, update, plugin files) and an **admin console**
  (everything else, in a browser);
- the console on its own address (`admin.lms.example.edu`), with its own login page (admin email and password),
  isolated from the student app, so that a problem in the student app cannot reach administration;
- a non-technical staff member can configure plugins, roles and units, people, and read the logs, without
  touching a terminal.

`vumlo-cli` stays. It is the console's terminal twin (same operations, same code), and it is how the first admin
is made and how a locked-out installation is recovered. It is no longer something a normal administrator has to learn.

## 2. Goals, non-goals, anti-goals

Goals
- A console origin with its own login, cookie, session lifetime and CSRF origin.
- Administration actions are refused to any session that did not come from the console.
- Plugin settings, unit roles (manager / viewer abilities), units, mail journal, plugin call log, audit log and
  people reachable from the console and from `vumlo-cli`, through one shared layer of operations.
- An admin can open any course from the console course list.

Non-goals (said out loud)
- A separate database, backend process or service for administration. Same binary, same database.
- TOTP or other second factors (they fit the console login later; not in this work).
- Changing files or containers on the host from the console. Editing `vumlo.toml`, `.env`, installing or removing
  plugin containers, updating the release stay in create-vumlo. The console changes data (rows), never the host.
- Custom roles beyond `manager` and `viewer` (ADR-0094 non-goal stays).
- Course-scope plugin settings: those belong to teachers and need a form in the student app. Separate work.
- Running `doctor` from the browser. It needs host access. Stays in the CLI.
- Granular permissions between admins. Every admin is equal.

Anti-goals (a failure even if it works)
- A student-app session of an admin user can change settings, people, plugins or roles.
- The console shows a secret (tokens, keys, passwords) to anyone. Existing rule, kept.
- The student app stops working if the console is down or not configured.
- A new per-plugin screen written by hand: plugin settings forms come from the manifest schema.

## 3. What exists (facts the design builds on)

- An admin is a row in `auth.users` with `role = 'admin'`, found by email. Login is email and password, or Google
  (ADR-0003, 0017, 0038). `vumlo-cli admin create` makes one.
- One cookie, `vumlo_session`: `HttpOnly; SameSite=Lax; Path=/`, no `Domain`, so it is bound to one host
  (`auth/src/routes/auth.rs`). Sessions are hashed random tokens in `auth.sessions`.
- CSRF: a non-read-only cookie request must carry `Origin` equal to `server.public_origin` or one of
  `server.allowed_origins` (`auth/src/routes/extract.rs`). Bearer requests skip it.
- Caddy serves web app and API under one origin: `/api/*` to the core, the rest to the web app (`deploy/Caddyfile`).
- `access::authorize(actor, Action)` decides everything. Admin-only actions test `is_admin` (the role).
- Plugin settings: typed spec, validation, sealing, audit, notification live in `plugins` but the set/unset flow is
  in `vumlo-cli/src/plugin_cmd.rs`. Unit roles: `GET/PUT /admin/unit-roles` and `/units` exist (ADR-0094, backend
  built); there is no CLI for them and no web page yet.
- `/admin/courses` lists all courses but its rows have no link; `/courses` lists only the person's own courses
  (`list_my_courses` starts from the user's own rows), so an admin has no path to a teacher's course although
  `ViewCourse` allows them.

## 4. Approaches considered

| | What | Verdict |
|---|---|---|
| A | Terminal wizard only (`plugin configure`) | Needed anyway (fallback, shared layer), not enough for non-technical staff |
| B | Declarative file + `apply` | Not chosen: files scare the target user; drift between file and database |
| C | Local-only web console behind an ssh tunnel | Not chosen: the owner wants staff to reach it by address |
| **D** | **Separate door, same system: own origin, cookie, audience on sessions, short lifetimes, admin-only password login** | **Chosen by the owner** |
| E | Separate service and database | Rejected: same data must be read, so little security gain for much operational cost |

The owner picked D and the console on a hostname behind the login. The shared operations layer (below) is built
first because the CLI wizard and the console both need it.

## 5. Design

### 5.1 One layer of operations, two faces

Operations move out of `vumlo-cli` and out of route handlers into the crate that owns the data. Each is two
functions, in line with CLAUDE.md sections 4 and 5: a service function that takes an `Actor`, authorizes, and calls
an inner function; and the inner function (connection only) that does validation, write, audit and notification.
The route handler calls the service function and the CLI, which has host access and no actor, calls the inner one
and writes its audit row with source `cli`. The handler (or command) opens the transaction and calls exactly one
writing function. The inner functions:

- `plugins`: set and unset a setting, list setting views for an install (institution scope), and
  seal/audit/notify as today (ADR-0072/0073). Route handlers and the CLI become thin.
- `org`: list units, set the abilities of a role, add/remove a unit role (the functions behind ADR-0094 routes).
- Audit entries made from the CLI keep source `cli`; from the console, the existing actor-based audit.

The CLI then adds: `plugin configure <id>` (one question per institution-scope setting from the spec:
description, current value, default; Enter keeps; hidden prompt for secrets when stdin is a terminal; re-asks on
invalid; answers are read line by line from stdin so it can be scripted), and `roles show|set <role>
--abilities a,b`. Unit tree and unit-role assignment are not CLI commands: they need the actor-based visibility
rules and are built as the console page (plan 3). In `plugins` the actor-free functions already exist, so the only
new one is `set_setting_text`; in `org` the abilities functions are split into an actor-checked service function
and an actor-free inner one. `doctor` lists installed plugins that have a setting with no default and no value and names
`plugin configure <id>`.

### 5.2 Audience on sessions

- Migration: `auth.sessions.audience text NOT NULL DEFAULT 'app' CHECK (audience IN ('app','console'))`.
- `Actor` carries the audience of its session. A session never changes audience.
- Administration actions require `role = admin` **and** `audience = console`. Which actions: everything an admin
  alone may do to *change or read configuration and people*: settings, people management, units' abilities,
  plugins (install, grants, settings, tokens), mail and plugin logs, audit log, admin reports, institution
  settings. Classification is an exhaustive match over `Action`, so a new action cannot compile without a choice.
- Course work stays as today for an admin's app session, confirmed by the owner: an admin reads **and edits** any
  course without being enrolled in it (`ViewCourse`, `ManageContent`, `EditCourse`, publish, archive, grading and
  the other actions inside a course keep their `is_admin` rule), each change audited by `audit_implicit`. Only
  actions that are not inside a course (configuration, people, plugins, unit abilities, logs, reports) need the
  console. The console links to courses in the student app and the admin follows the link with the app session.
- Every operation stays listed in `server/tests/authorization.rs`; a test asserts that an admin's `app` session
  gets `403` on each administration operation and that a `console` session gets through.
- Bearer tokens (apps) are sessions too and carry audience `app`; a bearer session cannot be `console`.

### 5.3 Console login and cookie

- `POST /api/v1/console/login {email, password}`, accepted only when the request's `Origin` is
  `server.admin_origin`. Accepts only users with `role = admin` and a password; a non-admin, unknown user and wrong password all
  give the same answer as a wrong password. No Google, no signup, no email-based password reset.
- Throttle: the existing `LoginThrottle` with its own key namespace so console failures do not lock the same
  person out of the app and the reverse.
- Cookie `vumlo_console`: `HttpOnly; SameSite=Strict; Path=/; Secure` (Secure unless `auth.cookie_secure=false`
  for local use), no `Domain`. It is only ever sent to the console host.
- Lifetime: 30 minutes idle (sliding), 8 hours absolute. Constants, not configuration.
- A temporary password (ADR-0017) works as in the app: the console login answers "change password first" and
  accepts nothing else until done.
- The login response sets a credential cookie, so it sends `Cache-Control: no-store`; the password is hashed
  (verified) in `spawn_blocking` before any transaction; passwords and the cookie are never logged (CLAUDE.md 11).
- Logout and "sign out everywhere" reuse the existing session endpoints.
- Resetting a forgotten admin password: `vumlo-cli admin set-password` (new; only `admin create` exists).

### 5.4 Origins and CSRF

- New required `server.admin_origin` (see 5.7).
- `check_origin` takes the audience of the session it is about to accept: `app` sessions need `public_origin` or
  `allowed_origins`; `console` sessions need exactly `admin_origin`. The cookie is read by name, so each host's
  cookie can only produce its own audience.
- CORS stays closed; the console surface reaches the API on its own origin through `/api/*` (same pattern as ADR-0013).

### 5.5 Console web surface

- One codebase (`web/`), one build. The console is the same Next app answering on the admin host with only the
  login page and the admin pages; on the app host `/admin` and the console login are not served. Selection by the
  request host at runtime (the admin host is given to the web process by one environment variable), so there is no
  second image. The desktop static export does not include the console.
- Pages: People, Terms, Groups, Institution, Audit (all moved from `/admin`; the course table and the owner dialog
  are gone from the admin area: an admin lists every course in the "All courses" tab of `/courses` in the student
  app, under the new course-oversight action `ViewAllCourses`, and changes an owner in course settings), plus new: **Plugins** (installed list, status, grants, settings form generated from
  the manifest spec, enable/disable/rotate token), **Units and roles** (the ADR-0094 PR B admin page: tree, roles on
  a unit, one switch per ability for `manager` and `viewer`), **Mail** (queue, journal), **Plugin calls**.
- The non-admin "Units" page for managers stays in the student app, as in ADR-0094.
- i18n strings in every shipped catalogue (en, uk).

### 5.6 Installer and deployment (create-vumlo)

- Install asks one new question: the admin address (flag `--admin-origin`; default for `http://localhost:3000`
  is `http://localhost:3001`; for a public origin, `admin.` in front of the host). It writes `server.admin_origin`
  and adds the console route to `deploy/Caddyfile` (a second site block: `/api/*` to the core, the rest to the web
  process with the admin host variable).
- `create-vumlo update` on an installation without `admin_origin` asks the same question and never writes it
  without the answer.
- The console is not exposed on a hostname by default for a localhost install; a public hostname is the owner's
  choice, protected by the login.

### 5.7 Upgrade (no compatibility mode: CLAUDE.md section 13)

- `server.admin_origin` is **required**. There is no "console off" state and no old behaviour kept behind a flag.
  A server without it does not start, and `vumlo-cli config check` names the key. Classified in the configuration
  schema (ADR-0054) by the builder: `public` only if the web app needs it in `client-config.json`, otherwise `private`.
- An existing installation is updated with `npx create-vumlo update`, which asks for the admin address and writes
  the key; until then the new release does not start. The release notes say this first.
- Enforcement is therefore always on: an admin's app session gets `403` on administration operations, and `/admin`
  is gone from the student app in the same release.
- The migration only adds a column with a default (`app`); existing sessions stay valid as app sessions, so every
  admin has to sign in to the console once.
- This is a change to existing `/api/v1/admin/*` behaviour for app sessions, which section 9 of CLAUDE.md
  ("changes are additive only") does not allow without a decision. It is the point of the feature, the project is
  before 1.0, and the owner is asked to accept it explicitly in the approval of this document. An ADR line records it.

## 6. Delivery

- **PR 0 (optional, independent):** rows of `/admin/courses` link to the course.
- **PR 1:** operations layer in `plugins` and `org`; CLI `plugin configure`, `units`, `roles`, `doctor` hint;
  tests for the moved operations (behaviour unchanged for existing commands).
- **PR 2:** audience column, `Actor.audience`, classification of `Action`, console login, cookie, throttle
  namespace, `admin_origin`, per-audience CSRF, `admin set-password`, authorization tests, OpenAPI,
  `sqlx prepare --all-features`.
- **PR 3:** console web surface and pages, host selection, i18n, Caddyfile and compose, create-vumlo question,
  docs (`docs/install.md`), ADR line, release notes with the breaking-change note.
- Each PR is on a branch, in small Conventional Commits, and passes the definition of done of CLAUDE.md section 14
  (fmt, clippy, tests at two thread counts, sqlx prepare, offline check, the `--features dummy` repeat, and
  `npm run typecheck` / `npm run build` for the web). No new crate or feature flag is needed (ADR-0014).

## 7. Left to the builder

- Exactly which `Action` variants are administration (5.2 gives the rule; the exhaustive match settles each).
- How the Next app selects the surface by host (middleware or route group) and what the dev setup looks like.
- Whether institution settings stored in `vumlo.toml` (name, accent, domains, language) are shown read-only in the
  console with a pointer to create-vumlo, and which ones are already editable through the existing institution API.
- Wording of the console login errors and the "change password first" screen.

## 8. Decisions that are mine, not the owner's (flag if wrong)

1. Console login by email, not a new username field.
2. Lifetimes 30 min idle / 8 h absolute as constants.
3. (Confirmed by the owner) An admin keeps read and edit access to any course from an app session, not enrolled;
   only administration outside a course needs the console.
4. No email-based password reset for admins; CLI only.
5. `admin_origin` is required and enforcement is always on (no compatibility mode, CLAUDE.md 13). The owner accepted
   this and the non-additive change to `/admin/*` for app sessions (5.7): the project is in development and there
   is one installation.
6. One web build selected by host, not two images.
