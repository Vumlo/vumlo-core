# Admin console, plan 2: session audience, console login, `admin_origin`

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An administrator signs in to a console origin with their own cookie and short session, and the server refuses every administration operation to any session that did not come from there.

**Architecture:** `auth.sessions` gets an `audience` (`app` or `console`) that travels through `access::Authenticated` into `Actor`. `access::authorize` reads it: the pure administration actions and the admin branch of the unit actions need a console admin; course oversight (`is_admin` inside a course) does not. The credential extractor learns a second cookie (`vumlo_console`), refuses a token presented through the wrong channel, and checks `Origin` against the origin of the audience. `POST /console/login` mints a console session only for an admin. `vumlo-cli admin set-password` is the recovery path.

**Tech Stack:** Rust (axum, sqlx, clap), the existing `server/tests/common` harness, `utoipa` OpenAPI and the generated web types.

**Spec:** `docs/superpowers/specs/2026-11-13-admin-console-design.md` (PR 2 of section 6; sections 5.2, 5.3, 5.4, 5.7). Plan 3 (console web surface, new admin APIs, deployment) follows.

## Amendments to the spec found while planning

- **`/admin/*` is not the only administration surface.** The units' admin branch (`ViewUnit`, `ManageUnits`, `ManageUnitCourses`) and `effective_unit_grant` (which hands an admin every ability on every unit) also give an admin power over structure. They are gated too, or an app session would keep it. Unit grants an admin holds by an explicit unit role still work in the app.
- **No HTTP routes exist yet for plugins, mail journal or plugin calls.** They are CLI-only today, so there is nothing to gate in this plan; plan 3 adds them and classifies their new `Action`s console-only (the exhaustive match forces the choice).
- **`touch_session` refreshes at most once an hour**, which cannot give a 30-minute idle limit. A console session refreshes at most once a minute; the other kinds keep an hour.
- **Two cookies can reach one host.** On `localhost` the web app (`:3000`) and the console (`:3001`) share cookies (cookies ignore ports). The credential rule below makes the request's `Origin` decide which cookie speaks, so an admin signed in to both is not broken and a write from the app origin never rides the console cookie.
- **Plan 2 is not releasable alone.** After it the student web app's `/admin` pages get `403` for everyone, and the console pages arrive in plan 3. Stack the two PRs and tag after plan 3.

## Global Constraints

- Work on a branch, never `main`; small Conventional Commits; push only when asked. Branch `feat/admin-console-door` from `feat/admin-console` (PR #67 not yet merged).
- Breaking change accepted by the owner, no compatibility mode (CLAUDE.md 13): `server.admin_origin` is required; an admin's app session gets `403` on administration operations.
- Lifetimes of a console session, constants, not configuration: **30 minutes idle (sliding), 8 hours absolute**.
- Cookie `vumlo_console`: `HttpOnly; SameSite=Strict; Path=/; Secure` (Secure unless `auth.cookie_secure = false`), no `Domain`. `vumlo_session` is unchanged.
- A bearer session is always `app`; a console session is never accepted as bearer or through `vumlo_session`, and an app session is never accepted through `vumlo_console`.
- Console login: only `role = admin`; non-admin, unknown user and wrong password are the same `401`; the password is verified in `spawn_blocking` before any transaction; the response carries `Cache-Control: no-store`; passwords, tokens and cookies are never logged or audited.
- A CLI command that writes the database writes an audit row with source `cli`; meta holds ids and field names only.
- Service functions that act for a user take an `Actor` and authorize; no role checks outside `access`; SQL only in `repo.rs`; nothing slow inside a transaction.
- Every new `server.*` key is classified (ADR-0054). `admin_origin` is `private` (plan 3 reclassifies it `public` if the console web app needs it in `client-config.json`).
- API changes additive only, except the accepted breaking change above. Regenerate `web/openapi.json` and `web/src/lib/api/schema.d.ts` (the same command Plan 1 used).
- Unit tests live in `foo/tests.rs`, never inline; server tests are real-server tests in `backend/crates/server/tests/`.
- Definition of done (CLAUDE.md 14), from `backend/`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` and `-- --test-threads=16` (run long tests one at a time, `CARGO_BUILD_JOBS=2`); `cargo sqlx prepare --workspace --check -- --all-targets` (run `prepare` with `--all-features`); `SQLX_OFFLINE=true cargo check --workspace --all-targets`; the clippy, test and offline check again with `--features dummy`; from `web/`: `npm run typecheck`, `npm run build`.

## Review Focus

1. **Cookie confusion on one host.** A request with both cookies and an `Origin` that is not `admin_origin` is authenticated by `vumlo_session`, never by `vumlo_console`; an unsafe request with the console cookie and the app origin is refused, not downgraded. (Task 3)
2. **Laundering a console token.** The raw token of a console session sent as `Authorization: Bearer` or as `vumlo_session` is `401`; an app token sent as `vumlo_console` is `401`. (Task 3)
3. **The console session lives 30 minutes idle and never past 8 hours**, even when used every minute; an hour-old `last_seen_at` is not the refresh threshold for it. (Task 2)
4. **A non-admin with the right password learns nothing:** same status, same body, same throttle counting as a wrong password; and console failures do not lock the same person out of the app, nor the reverse. (Task 4)
5. **An admin demoted or disabled while holding a console session loses it at once** (`401`), and `admin set-password` and the admin reset sign every session of that user out, console ones included. (Tasks 3, 6)
6. **An admin's app session still reads and edits any course, audited, and gets no implicit grant on units**; a unit role the admin holds explicitly still works. (Task 1, Task 5)
7. **`admin_origin` equal to `public_origin` (or listed in `allowed_origins`) refuses to start**, naming the key. (Task 0)

## File structure

```
backend/migrations/0056_console_sessions.sql                new  audience column
backend/crates/config/src/{settings.rs,load.rs,tests.rs}    admin_origin, its checks
backend/crates/access/src/{lib.rs,actor.rs,extract.rs,test_support.rs,tests.rs}   Audience, gating
backend/crates/auth/src/{model.rs,repo.rs,service.rs}       session audience, lifetimes, console login, set-password
backend/crates/auth/src/routes/{extract.rs,auth.rs,mod.rs}  credential channels, console login route
backend/crates/api-types/src/lib.rs                         ConsoleLoginRequest
backend/crates/server/src/app.rs                            Authenticated mapping
backend/crates/server/tests/common/mod.rs                   console harness
backend/crates/server/tests/console.rs                      new: login, channels, lifetimes
backend/crates/server/tests/authorization.rs                the audience matrix
backend/crates/vumlo-cli/src/{admin.rs,main.rs,config_cmd.rs}   admin set-password
deploy/vumlo.example.toml, docs/, CHANGELOG.md, docs/decisions/in-progress.md
```

---

### Task 0: `server.admin_origin`, required and checked

**Files:**
- Modify: `backend/crates/config/src/settings.rs` (`ServerSettings`), `backend/crates/config/src/load.rs` (the origin checks near `check("server.public_origin", ...)`), `backend/crates/config/src/tests.rs`, `backend/crates/vumlo-cli/src/config_cmd.rs`, `deploy/vumlo.example.toml`, `backend/crates/vumlo-cli/tests/common/mod.rs` (its TOML), `backend/crates/server/tests/common/mod.rs` (its `ServerSettings`, add `pub const ADMIN_ORIGIN: &str = "https://admin.lms.test"`), `backend/crates/auth/src/routes/extract/tests.rs` (its `server()` literal), and every other literal the compiler names.
- Test: `backend/crates/config/src/tests.rs`

**Interfaces:**
- Produces: `ServerSettings.admin_origin: String`, normalised like `public_origin` (no trailing slash, no path). Tasks 3, 4 and the test harness read it.

- [ ] **Step 1: Write the failing tests** (names; each loads a TOML through the same entry point as the neighbouring origin tests):
  - `a_missing_admin_origin_is_named_in_the_error` — the error text contains `server.admin_origin`.
  - `the_admin_origin_is_normalised_like_the_public_one` — `"https://admin.lms.test/"` loads as `"https://admin.lms.test"`; a value with a path is an error naming the key.
  - `the_admin_origin_must_differ_from_the_public_origin` — equal values give an error naming both keys.
  - `the_admin_origin_is_not_a_trusted_frontend` — an `allowed_origins` entry equal to `admin_origin` is an error.
- [ ] **Step 2: Run** `cargo test -p vumlo-config admin_origin`. Expected: FAIL (field unknown / no error).
- [ ] **Step 3: Implement** the field (`#[schemars(extend("x-visibility" = "private"))]`, doc comment: the origin of the admin console; cookie requests to the console must carry exactly this `Origin`), the `check("server.admin_origin", ...)` call, and the two cross-key errors in the same validation pass as the other server checks. Add `admin_origin` to `deploy/vumlo.example.toml` with a comment (default for the localhost trial: `http://localhost:3001`) and print it in `config show` as `admin URL`. Fix every struct literal and TOML fixture until `cargo check --workspace --all-targets` is clean.
- [ ] **Step 4: Run** `cargo test -p vumlo-config` and `cargo test -p vumlo-cli config`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(config): server.admin_origin is required (ADR-0095)`.

### Task 1: `Audience` in `access`, and what needs the console

**Files:**
- Modify: `backend/crates/access/src/{lib.rs,actor.rs,extract.rs,test_support.rs}`, `backend/crates/server/src/app.rs` (the one `access::Authenticated {` literal: it sets `audience: Audience::App` in this task, which is true because no console session exists yet, and Task 2 replaces it with `session.audience`), `backend/crates/module-api/src/state.rs` if it constructs one.
- Test: `backend/crates/access/src/tests.rs`

**Interfaces:**
- Produces: `pub enum Audience { App, Console }` (`Copy`, `Debug`, `PartialEq`, `Eq`, `text_enum!` with `"app"` / `"console"`), re-exported from `access`; `Authenticated.audience: Audience`; `Actor::audience(&self) -> Audience` (public: handlers choose the cookie to clear by it); `Actor::for_tests(user_id, role, session_id)` builds an **App** actor; `Actor::for_tests_in(user_id, role, session_id, audience)`.
- Rule in `authorize`: `let console_admin = is_admin && actor.audience() == Audience::Console;`. These use `console_admin` instead of `is_admin`: `ManageUsers`, `ViewInstitutionSettings`, `ManageAppDownloads`, `ViewAdminReports`, `ManageTerms`, `ManageGroups`, `ViewGroups`, `ManageUnitRoleAbilities`, and the `is_admin` arm of `ViewUnit`, `ManageUnitCourses`, `ManageUnits`. Everything with a `CourseAccess`, `ViewAllCourses`, `CreateCourse`, `ViewOwnCourses`, `ViewTerms`, `ViewEditorProfiles`, `ViewOwnUnits`, `ViewFormerGrades`, `ManageOwnAccount` and `Module { .. }` keep `is_admin` (course oversight and ordinary use; a module permission is a feature, not administration). `effective_unit_grant` gives an admin the full grant only for the console; an app admin has `held`, like a teacher. Add one comment above the match stating this rule, so a new action's author sees it.

- [ ] **Step 1: Write the failing tests** in `access/src/tests.rs` (use `for_tests_in`):
  - `an_admin_app_session_is_refused_every_administration_action` — table over the eight actions above plus `ViewUnit(None)`, `ManageUnits(None)`, `ManageUnitCourses(None)`: `Denied` for an admin in `App`, `Ok` in `Console`, `Denied` for a teacher in either audience.
  - `an_admin_app_session_keeps_course_oversight` — `ViewAllCourses`, and `ViewCourse`, `EditContent`, `EditCourse`, `ManageMembership`, `DeleteCourse` for a course the admin is not on: `Ok` in both audiences.
  - `an_admin_app_session_has_no_implicit_unit_grant` — `effective_unit_grant` is `None` for an App admin with `held = None`, `Some(full)` for Console, `held` for an App admin who holds a role.
  - `an_admin_with_an_explicit_unit_grant_acts_through_it_in_the_app` — `ViewUnit(Some(grant))` is `Ok` in `App`.
- [ ] **Step 2: Run** `cargo test -p access audience`. Expected: FAIL (no `Audience`).
- [ ] **Step 3: Implement** as in the Interfaces block. Existing `access` tests that exercise admin-only actions switch to `for_tests_in(.., Console)`; do not weaken any assertion.
- [ ] **Step 4: Run** `cargo test -p access`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(access): administration needs a console session (ADR-0095)`.

### Task 2: Session audience and lifetimes in `auth`

**Files:**
- Create: `backend/migrations/0056_console_sessions.sql`
- Modify: `backend/crates/auth/src/{model.rs,repo.rs,service.rs}`, `backend/crates/server/src/app.rs`
- Test: `backend/crates/auth/src/service/tests.rs`, `backend/crates/auth/src/model/tests.rs`

**Interfaces:**
- Consumes: `access::Audience` (Task 1).
- Produces:
  - SQL: `ALTER TABLE auth.sessions ADD COLUMN audience text NOT NULL DEFAULT 'app' CHECK (audience IN ('app','console'))` and `CHECK (audience = 'app' OR kind = 'web')` (only a browser session can be console).
  - `service::create_session(conn, login: &VerifiedLogin, kind: SessionKind, audience: Audience) -> Result<NewSession, AuthError>`.
  - `service::Authenticated { .., audience: Audience, kind: SessionKind, touch: Option<(SessionKind, Audience)> }` (keep the existing fields); `service::touch_session(conn, session_id, kind, audience)`.
  - `model::Lifetimes { initial: Duration, slide: Option<Duration>, cap: Option<Duration>, refresh_gap: Duration }` and `SessionKind::lifetimes(self, audience: Audience) -> Lifetimes`; for `(Web, Console)`: initial 30 min, slide 30 min, cap 8 h, refresh gap 1 min; every other pair keeps today's numbers and a one-hour gap (`initial_lifetime` / `sliding_lifetime` are replaced by it).
  - `repo::find_session` returns the audience and `idle_secs` (seconds since `last_seen_at`); staleness is decided in Rust as `idle >= refresh_gap`; `repo::touch_session(conn, session_id, slide_secs: Option<i64>, cap_secs: Option<i64>, gap_secs: i64)` sets `expires_at = LEAST(now() + slide, created_at + cap)` when both are present.
  - `find_session` also requires `u.role = 'admin'` for a console session, so a demoted user's console session stops resolving.

- [ ] **Step 1: Write the failing tests** (DB tests with `db_test`, backdating rows by SQL):
  - `a_console_session_expires_after_thirty_minutes_idle` — `expires_at` = now + 30 min on creation; with `expires_at` in the past it does not resolve.
  - `a_console_session_slides_but_never_past_eight_hours` — `created_at` set to 7 h 59 min ago and `last_seen_at` two minutes ago, after `touch_session` `expires_at` equals `created_at + 8 h`, not now + 30 min.
  - `a_console_session_is_refreshed_after_a_minute_not_an_hour` — `last_seen_at` 2 minutes old makes `authenticate` return `touch: Some(..)`; an app web session of the same age does not.
  - `a_console_session_of_a_demoted_admin_does_not_resolve` — change the user's role to `teacher`; `authenticate` answers `Unauthenticated`.
  - `the_database_refuses_an_app_client_console_session` — inserting `kind = 'app', audience = 'console'` violates the check.
- [ ] **Step 2: Run** `cargo test -p auth session`. Expected: FAIL (no column / functions).
- [ ] **Step 3: Implement** the migration (also touch nothing else in the table), the model, repo and service changes above, and map `audience` in `server/src/app.rs`. Every existing caller of `create_session` passes `Audience::App`. Run `cargo sqlx prepare --workspace -- --all-targets --all-features` and commit `.sqlx/`.
- [ ] **Step 4: Run** `cargo test -p auth`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(auth): sessions have an audience and the console's lifetimes (ADR-0095)`.

### Task 3: Credential channels and per-audience CSRF

**Files:**
- Modify: `backend/crates/auth/src/routes/extract.rs`, `backend/crates/auth/src/routes/mod.rs` (export `CONSOLE_COOKIE_NAME`)
- Test: `backend/crates/auth/src/routes/extract/tests.rs`

**Interfaces:**
- Consumes: `ServerSettings.admin_origin` (Task 0), `service::Authenticated.audience` (Task 2).
- Produces:
  - `pub const CONSOLE_COOKIE_NAME: &str = "vumlo_console";`
  - `enum Channel { Bearer, SessionCookie, ConsoleCookie }` with `fn audience(self) -> Audience` (`ConsoleCookie` → `Console`, others → `App`).
  - `fn credential(headers: &HeaderMap, server: &ServerSettings) -> Result<(Channel, SessionToken), AuthError>`. Order: `Authorization: Bearer` wins; else the console cookie when present and the request's `Origin` is absent or equals `admin_origin`; else `vumlo_session`. A credential present but malformed is rejected, not skipped.
  - `check_origin(method, headers, server, audience: Audience) -> Result<(), AuthError>`: read-only methods pass; `App` accepts `public_origin` and `allowed_origins`; `Console` accepts exactly `admin_origin`.
  - In `authenticate_request`: after the lookup, `authenticated.audience != channel.audience()` is `Unauthenticated`.

- [ ] **Step 1: Write the failing tests** (unit tests on `credential` and `check_origin`; the session-level ones in Task 4's server tests):
  - `the_console_cookie_is_used_without_an_origin_or_with_the_admin_origin`
  - `the_console_cookie_is_ignored_when_the_origin_is_the_app` — with both cookies and `Origin: <public_origin>` the channel is `SessionCookie`.
  - `a_bearer_header_wins_over_both_cookies`
  - `a_malformed_console_cookie_is_rejected_not_skipped`
  - `console_requests_need_exactly_the_admin_origin` — `public_origin`, an `allowed_origins` entry, the admin origin with a trailing slash, `null` and none are `OriginMismatch`; the admin origin passes for `POST`, `PATCH`, `PUT`, `DELETE`.
  - `app_requests_no_longer_accept_the_admin_origin` — for `Audience::App`, `admin_origin` is `OriginMismatch`.
  - Port the existing origin tests to the four-argument `check_origin` unchanged in meaning.
- [ ] **Step 2: Run** `cargo test -p auth extract`. Expected: FAIL (signature, no console cookie).
- [ ] **Step 3: Implement** the Interfaces block. Keep the module's doc comment true.
- [ ] **Step 4: Run** `cargo test -p auth`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(auth): the console cookie, and an origin check per audience (ADR-0095)`.

### Task 4: Console login and logout

**Files:**
- Modify: `backend/crates/auth/src/{service.rs,routes/auth.rs,routes/mod.rs}`, `backend/crates/api-types/src/lib.rs`, `backend/crates/server/tests/common/mod.rs`
- Create: `backend/crates/server/tests/console.rs`
- Test: `backend/crates/server/tests/console.rs`, `backend/crates/auth/src/service/tests.rs`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces:
  - `api_types::ConsoleLoginRequest { email: String, password: String }` (`garde`-validated like `LoginRequest`; `Debug` redacts the password) and `POST /api/v1/console/login` answering `200` with `api_types::User`, `401`, `403` (origin), `422`, `429`; tag `auth`.
  - `service::lookup_login(conn, throttle, email, audience: Audience)`; `ThrottleKey::new(email)` stays for the app and mail, `ThrottleKey::console(email)` hashes `"console:"` + the normalised email; `service::complete_login(throttle, gate, attempt, password)` returns `InvalidCredentials` for a non-admin of a console attempt (after the password check, so timing and counting match a wrong password).
  - `routes::console_cookie(token: &str, max_age_secs: u64, secure: bool) -> Result<HeaderValue, ApiError>` (`vumlo_console`, `HttpOnly; SameSite=Strict; Path=/`, `Max-Age` 8 h at login).
  - The route sits in its own router with its own `client_limiter()` allowance, next to the existing login router.
  - `logout` clears the cookie of the actor's audience (`console_cookie("", 0, ..)` for `Console`).
  - Harness: `common::login_console(http, email, password) -> String` (POST with `Origin: ADMIN_ORIGIN`, returns the `vumlo_console` token) and `Http::console(self, token) -> Self` (sets `Cookie: vumlo_console=<token>` and `Origin: ADMIN_ORIGIN`).

- [ ] **Step 1: Write the failing tests** in `server/tests/console.rs`:
  - `an_admin_signs_in_to_the_console_and_gets_the_console_cookie` — `200`, `Set-Cookie` starts `vumlo_console=`, has `HttpOnly`, `SameSite=Strict`, `Path=/`, `Secure` (the harness has `cookie_secure = true`), no `Domain`; `Cache-Control: no-store`; the session row has `audience = 'console'`, `kind = 'web'`.
  - `a_wrong_password_an_unknown_email_and_a_non_admin_look_the_same` — three requests, equal status and body (`401`); the non-admin has the right password.
  - `the_console_login_needs_the_admin_origin` — the public origin, another origin and no origin are `403` and create no session.
  - `console_failures_do_not_lock_the_app_login_and_the_reverse` — exhaust the throttle on one side, the other still signs in.
  - `a_temporary_password_admin_can_only_change_it` — console login `200` with `must_change_password`, `GET /me` ok, `GET /admin/users` is `403` "the password must be changed first".
  - `logout_clears_the_console_cookie_and_deletes_the_session` — `Set-Cookie: vumlo_console=; Max-Age=0`, token no longer resolves.
  - `a_console_token_is_not_a_bearer_token_and_not_an_app_cookie` — the raw console token as `Authorization: Bearer` and as `vumlo_session` is `401`; an app bearer token sent as `vumlo_console` is `401` (Review Focus 2).
  - `a_demoted_admin_loses_the_console_session_at_once` — change the role in SQL, next call is `401` (Review Focus 5).
  - `a_write_with_the_console_cookie_from_the_app_origin_is_refused` — `POST` with the console cookie and `Origin: ORIGIN` is not served as the admin (`401` or `403`); with both cookies and the app origin the app session answers (Review Focus 1).
- [ ] **Step 2: Run** `cargo test -p server --test console`. Expected: FAIL (404 on `/console/login`).
- [ ] **Step 3: Implement** the Interfaces block. The handler: `check_origin(&Method::POST, &headers, &server, Audience::Console)`; `lookup_login` before the hash, `complete_login` (hashing), then one short transaction for `create_session(.., SessionKind::Web, Audience::Console)`. Add the harness helpers. Add the console cookie to the OpenAPI security schemes. Regenerate `web/openapi.json` and `schema.d.ts`; fix any web type error (`npm run typecheck`).
- [ ] **Step 4: Run** `cargo test -p server --test console` and `cargo test -p auth`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(auth): console login and logout (ADR-0095)`.

### Task 5: Enforcement end to end, and the test harness

**Files:**
- Modify: `backend/crates/server/tests/common/mod.rs`, `backend/crates/server/tests/authorization.rs`, every test file that calls an administration route with an admin token and fails (the compiler and the suite name them: `admin_reports.rs`, `org.rs`, `identity.rs`, `identity_audit.rs`, `institution.rs`, `groups_roster.rs`, `units.rs`, `courses.rs`, `course_roles.rs`, and others).
- Test: `backend/crates/server/tests/authorization.rs`

**Interfaces:**
- Consumes: Task 4 harness.
- Produces: `Env::user(email, Role::Admin)` signs the admin in to the **console** and registers the token in a process-wide set in `common`; `Http::bearer(token)` sends a registered console token as the console cookie plus `Origin: ADMIN_ORIGIN`, any other token as `Authorization: Bearer`. About 370 call sites keep their shape and now exercise the console path. `Env::app_admin(email) -> Account` signs an admin in as an app client (what a student-app session of an admin is), for the tests that need one.

- [ ] **Step 1: Write the failing tests** in `authorization.rs`, beside the existing matrix:
  - `an_admin_app_session_is_refused_every_administration_operation` — reuse the existing administration rows of the matrix (the `/admin/*` list, `/admin/unit-roles`, `PUT /admin/unit-roles/{role}`, institution, users, groups overview, term stats, audit, `GET /units/{id}` and `POST /units` for a unit the admin holds no role in): `403` with `Env::app_admin`, success with the console admin. A row added to the matrix cannot skip this test.
  - `an_admin_app_session_still_works_inside_a_course` — with `app_admin`: read the course, edit content, change the roster, archive; each answers as for the console admin, and an audit row with `via = admin` exists for the write. `GET /admin/courses` (all courses) works.
  - `an_admin_with_a_unit_role_uses_it_from_the_app` — grant the admin `manager` on a unit; `app_admin` reads that unit and still gets `403` on a sibling unit.
- [ ] **Step 2: Run** `cargo test -p server --test authorization`. Expected: FAIL (`app_admin` missing; an app admin still passes).
- [ ] **Step 3: Implement** the harness, then run the whole workspace once: `cargo test --workspace > $SCRATCH/t.log 2>&1` (one at a time, `CARGO_BUILD_JOBS=2`) and fix each failing test file by giving the admin the right credential (`Env::user` already does; a test that logged an admin in by hand with `login_app` or `seed_and_login` and then calls an administration route switches to `Env::user`). Do not edit an assertion to hide a `403`: if a test now expects admin power from an app session, that test is wrong about the product.
- [ ] **Step 4: Run** `cargo test --workspace` and again with `-- --test-threads=16`. Expected: PASS.
- [ ] **Step 5: Commit** — `test(server): administration needs a console session (ADR-0095)`.

### Task 6: `vumlo-cli admin set-password`

**Files:**
- Modify: `backend/crates/auth/src/{service.rs,repo.rs}` (`set_admin_password_from_cli`), `backend/crates/vumlo-cli/src/{admin.rs,main.rs}`
- Test: `backend/crates/vumlo-cli/tests/admin.rs` (create if the CLI tests have no admin file), `backend/crates/auth/src/service/tests.rs`

**Interfaces:**
- Produces: `auth::set_admin_password_from_cli(conn, email: &str, password: HashedPassword) -> Result<Uuid, AuthError>`: the user must exist with `role = admin` and status active (else `UserNotFound`, same message for "no such user" and "not an admin"), replaces the password hash, clears `must_change_password`, deletes every session of the user, writes `events::audit_cli` with action `user.password_set` and `meta` `{}`. CLI `vumlo-cli admin set-password <email> [--password-stdin] [--dry-run]`, same prompt, policy and stdin rules as `admin create` (`read_password`, `PasswordPolicy`); hash before the transaction; `--dry-run` checks the account exists and writes nothing.

- [ ] **Step 1: Write the failing tests:**
  - `set_password_replaces_the_password_and_signs_the_admin_out` — console login with the old password then the new one; the old console and app sessions are gone (Review Focus 5).
  - `set_password_audits_with_source_cli_and_no_secret` — one `user.password_set` row, source `cli`, no password or hash in `meta`.
  - `set_password_refuses_a_teacher_and_an_unknown_email_the_same_way` — same message, nothing changed.
  - `set_password_dry_run_changes_nothing` and `set_password_rejects_a_weak_password` (policy message, nothing changed).
- [ ] **Step 2: Run** `cargo test -p vumlo-cli --test admin set_password`. Expected: FAIL (unknown subcommand).
- [ ] **Step 3: Implement** the Interfaces block; reuse `read_password`; `main.rs` gets the `SetPassword { email, password_stdin, dry_run }` variant and its help text ("recovery for a locked-out administrator").
- [ ] **Step 4: Run** `cargo test -p vumlo-cli` and `cargo test -p auth`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(vumlo-cli): admin set-password recovers a locked-out administrator (ADR-0095)`.

### Task 7: Docs, OpenAPI and the definition of done

**Files:**
- Modify: `docs/decisions/in-progress.md` (extend the ADR-0095 line: audience column, lifetimes, cookie, per-audience origin, the unit gating), `CHANGELOG.md` (Unreleased: **breaking** — `server.admin_origin` required, administration refused to app sessions, console login, `admin set-password`), `docs/install.md` (the new key in the config table and the localhost default), `ARCHITECTURE.md` (the auth section: audience), `web/openapi.json`, `web/src/lib/api/schema.d.ts`.

- [ ] **Step 1: Write** the documentation changes above; the CHANGELOG entry says first, in one line, that an installation must add `server.admin_origin` before this release starts and that student-app `/admin` pages stop working until the console ships in the same release.
- [ ] **Step 2: Run the full definition of done** (Global Constraints), reading each exit code; also `vumlo-cli config check` against the example file. Expected: all exit 0.
- [ ] **Step 3: Self-review** the branch against Review Focus 1–7 with the package from `review-package`, then ledger the result (a fresh reviewer if the owner allows one).
- [ ] **Step 4: Commit** — `docs: the admin console door (ADR-0095)`.

---

## Left for plan 3 (named so nothing is lost)

- The console web surface: host selection in the Next app, the login page, the pages moved from `/admin`, removal of `/admin` from the student app, the "change password first" screen, i18n.
- HTTP routes for plugins (list, grants, settings from the manifest spec, enable, rotate token), unit tree and unit roles (the ADR-0094 PR B admin page), mail journal and plugin calls, each with a new console-only `Action`.
- `server.admin_origin` in `client-config.json` if the console app needs it; the Caddy block, compose, `docs/install.md` walk-through, and the `create-vumlo` question (`--admin-origin`, `update` asking once).
- Release notes with the breaking-change note, tagged only after plan 3.
