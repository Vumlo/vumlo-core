# Admin console, plan 3: the console's web surface, its API, and deployment

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An administrator opens `admin.lms.example.edu`, signs in, and runs the installation from a browser: people, groups, terms, institution, plugins (settings and logs), units and unit roles, mail and the audit log, with nothing of it left in the student app.

**Architecture:** One Next app, one build. A middleware selects the surface by the request's `Host`: on the admin host only `/console/*` (and `/api`, `/_next`, brand files) is served, on every other host `/console/*` is `404`. The console is a route group with its own shell and sign-in against `POST /console/login` (plan 2). The API gets the routes the new pages need, in the crates that own the data, each behind a new console-only `Action`. Deployment gets a second Caddy site block and one more published port; `create-vumlo` asks for the admin address.

**Tech Stack:** Rust (axum, utoipa), Next.js 16 and React 19, MSW mocks, Playwright (system Chrome), Caddy, `create-vumlo` (Node).

**Spec:** `docs/superpowers/specs/2026-11-13-admin-console-design.md` (PR 3 of section 6; sections 5.5 and 5.6). Follows plan 2 (`docs/superpowers/plans/2026-11-13-admin-console-plan-2-audience-and-console-login.md`), on which it is stacked.

## Amendments to the spec found while planning

- **No token rotation in the console.** Spec 5.5 lists "rotate token" among the plugin actions, but its anti-goal says the console never shows a secret, and a new token is shown once. The token also has to be put into the plugin's environment on the host, which is create-vumlo's side (non-goal: the console changes rows, never the host). Rotation stays `vumlo-cli plugin rotate-token`. Also not in the console: install and remove (they start and stop containers).
- **`server.admin_origin` becomes `public`** in the configuration schema: the web build needs it for the middleware and for the student app's "Open the admin console" link, and it is only an address. (Plan 2 classified it `private`; this plan reclassifies it, as plan 2 said it would.)
- **One process serves both addresses.** The container publishes a second port (`3001`) for a trial, and Caddy routes the admin hostname to the same upstream with `Host` kept, so the middleware sees which door was used. Local development has no second hostname; the dev story below uses `127.0.0.1` against `localhost`, which browsers keep apart for cookies.
- **Units and unit roles need no new API**: `GET/POST /units`, `/units/{id}/roles`, `/units/{id}/candidates` and `/admin/unit-roles` exist (ADR-0094). Only their page is new.
- **The term list** is the existing terms half of the old admin courses page; it moves as "Terms".

## Global Constraints

- Work on a branch, never `main`; small Conventional Commits; push only when asked. Branch `feat/admin-console-web` from `feat/admin-console-door`.
- Plan 2's rule stays: every new administration `Action` is checked with `console_admin` in `access::authorize`; a test asserts an admin's app session gets `403` on each new route and the console session passes.
- The console never shows a secret: no setting value of a secret setting, no token, no password, in any response or page; audit meta carries ids and keys only. Responses that carry a credential send `Cache-Control: no-store`.
- A write from the console is audited with the actor (`events::audit`), not with the CLI source.
- SQL only in `repo.rs`; service functions take an `Actor`; routes live in the crate that owns the data; no new crate, no new feature flag (ADR-0014).
- The desktop static export does not contain the console; the student app has no `/admin` pages left.
- Web: colours only from `styles/tokens.css` (`npm run lint` colour check); every string in `en.ts` and `uk.ts`; a page works at phone width; no new dependency.
- Definition of done (CLAUDE.md 14), from `backend/`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` and `-- --test-threads=16` (one run at a time, `CARGO_BUILD_JOBS=2`); `cargo sqlx prepare --workspace --check -- --all-targets` (prepare with `--all-features`); `SQLX_OFFLINE=true cargo check --workspace --all-targets`; clippy, test and offline check again with `--features dummy`; from `web/` after `vumlo-cli brand apply`: `npm run typecheck`, `npm run build`, `npm run build:desktop`; the Playwright specs (system Chrome; the scratch config is outside the repo).

## Review Focus

1. **Surface by host.** On the admin host `/` and every student route lead to the console and serve no student page; on the app host `/console/*` is `404`; `/api/*` works on both. (Task 4)
2. **A stale or student session at the console.** A `401` on any console call, or an app-only session, lands on the console sign-in, never on the student sign-in; sign out clears the console cookie only. (Task 4)
3. **No secret on the wire.** A secret plugin setting is accepted by `PUT` and never returned (`is_set` only); the audit entry names the key, not the value. (Task 1)
4. **A disabled plugin through the console behaves as through the CLI**: its queued mail is cancelled in the same transaction, deliveries wait. (Task 1)
5. **The mail page shows no address**, only ids, plugin, kind, state and counts. (Task 2)
6. **The desktop export has no console**, and the student shell has no `/admin` link, only the "Open the admin console" link for an administrator. (Tasks 4, 5)
7. **An installation updated without `admin_origin`** is told so by `create-vumlo update` and the key is never written without an answer. (Task 8)

## File structure

```
backend/crates/access/src/lib.rs              ViewPlugins, ManagePlugins, ViewMail, ManageMail
backend/crates/config/src/{client.rs,settings.rs}   admin_origin public
backend/crates/plugins/src/{admin.rs,routes/admin.rs}   admin service + routes (new files)
backend/crates/mail/src/{admin.rs,routes admin}          admin service + routes
backend/crates/api-types/src/lib.rs           the DTOs
backend/crates/server/src/app.rs, tests/{console_plugins.rs,console_mail.rs,authorization.rs}
web/src/middleware.ts                         surface by host (proxy.ts if Next 16.4 names it so)
web/src/app/console/**                        layout, login, change-password, pages
web/src/components/console/**                 moved from components/admin, plus the new pages
web/src/lib/api/hooks/{console.ts,plugins-admin.ts,mail.ts}, web/src/mocks/handlers/console.ts
web/src/lib/i18n/{en.ts,uk.ts}, web/e2e/console*.spec.ts
deploy/Caddyfile, deploy/compose.yaml, compose.yaml, docs/install.md, CHANGELOG.md
create-vumlo/{index.mjs,test.mjs}             (its own repository)
```

---

### Task 0: `admin_origin` in the client configuration

**Files:**
- Modify: `backend/crates/config/src/{settings.rs,client.rs,client/tests.rs}`; regenerate `web/src/generated/client-config.{json,d.ts}` with `vumlo-cli brand apply`; `desktop/scripts/config.test.mjs` fixture if it lists the public server block.
- Test: `backend/crates/config/src/client/tests.rs`

**Interfaces:**
- Produces: `PublicServer.admin_origin: String`; the key is `x-visibility = "public"`; `web/src/lib/clientConfig.ts` exposes it.

- [ ] **Step 1: Write the failing test** `the_public_config_carries_the_admin_origin_and_no_other_private_key` — the client config JSON for a loaded settings value contains `public.server.admin_origin` and still contains none of `bind_addr`, `client_ip_header`, `allowed_origins`.
- [ ] **Step 2: Run** `cargo test -p vumlo-config client`. Expected: FAIL.
- [ ] **Step 3: Implement** the reclassification and the field; update the config tests that assert the key is private; regenerate the generated files and fix the web type errors.
- [ ] **Step 4: Run** `cargo test -p vumlo-config` and `npm run typecheck`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(config): the admin origin is public (ADR-0095)`.

### Task 1: Plugin administration API

**Files:**
- Modify: `backend/crates/access/src/lib.rs` (`Action::ViewPlugins`, `Action::ManagePlugins`, both `console_admin`), `backend/crates/api-types/src/lib.rs`, `backend/crates/plugins/src/lib.rs`, `backend/crates/server/src/app.rs`
- Create: `backend/crates/plugins/src/admin.rs` (service wrappers), `backend/crates/plugins/src/routes/admin.rs` if the crate already has a `routes` folder (else beside `routes.rs`)
- Test: `backend/crates/server/tests/console_plugins.rs`, `backend/crates/access/src/tests.rs`, `backend/crates/server/tests/authorization.rs` (the new rows)

**Interfaces:**
- Consumes: `plugins::{list, get, grant, set_status, list_settings, set_setting_text, unset_setting, calls}` (actor-free, exist).
- Produces, each taking `&Actor` and authorizing first: `plugins::admin_list(conn, actor) -> Vec<InstallSummary>`, `admin_get`, `admin_grant(conn, actor, plugin_id, &[Scope], &CourseScope)`, `admin_set_status`, `admin_settings(conn, actor, plugin_id) -> Vec<SettingView>`, `admin_set_setting(conn, actor, plugin_id, key, text)`, `admin_unset_setting`, `admin_calls`; each write calls `events::audit(conn, actor, entry)` with `{plugin, key}` meta and never a value.
- Routes (tag `admin`, `security(("console" = []))`): `GET /admin/plugins`, `GET /admin/plugins/{id}`, `PUT /admin/plugins/{id}/status` `{status}`, `PUT /admin/plugins/{id}/grants` `{scopes, courses}`, `GET /admin/plugins/{id}/settings`, `PUT /admin/plugins/{id}/settings/{key}` `{value: string}`, `DELETE` the same, `GET /admin/plugins/{id}/calls?since=&limit=`. A secret setting view has `is_set` and no `value`.

- [ ] **Step 1: Write the failing tests:**
  - `an_admin_app_session_is_refused_the_plugin_routes_and_the_console_is_not` — every route above: app admin `403`, teacher `403`, console admin not `401`/`403`.
  - `the_console_lists_installs_with_status_scopes_and_pending_deliveries`.
  - `a_secret_setting_is_accepted_and_never_returned` — `PUT` a secret, `GET` shows `is_set: true` and no `value`; the audit row has the key and not the value.
  - `an_invalid_value_is_a_422_that_names_the_setting` and `a_setting_of_course_scope_is_refused_here`.
  - `disabling_a_plugin_cancels_its_queued_mail_and_enabling_does_not_bring_it_back` (same expectation as the CLI test).
  - `the_calls_report_counts_by_route_and_answer`.
  - In `access`: the new actions are `Denied` for an app admin and `Ok` for a console admin.
- [ ] **Step 2: Run** the new test files. Expected: FAIL (404).
- [ ] **Step 3: Implement** the Interfaces block; register the router in `server/src/app.rs`; add the rows to the authorization matrix and the public-list test; regenerate `web/openapi.json` and `schema.d.ts`; `cargo sqlx prepare --all-features`.
- [ ] **Step 4: Run** the new tests, `cargo test -p plugins`, `cargo test -p server --test authorization`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(plugins): administer plugins from the console (ADR-0095)`.

### Task 2: Mail administration API

**Files:**
- Modify: `backend/crates/access/src/lib.rs` (`ViewMail`, `ManageMail`, `console_admin`), `backend/crates/api-types/src/lib.rs`, `backend/crates/mail/src/lib.rs`, `backend/crates/server/src/app.rs`
- Create: `backend/crates/mail/src/admin.rs`, its routes
- Test: `backend/crates/server/tests/console_mail.rs`, `authorization.rs`

**Interfaces:**
- Consumes: `mail::{status, list, retry}` and `mail::{Filter, JournalEntry, RetryScope}` (exist; the CLI uses them).
- Produces: `mail::admin_status(conn, actor)`, `admin_list(conn, actor, &Filter, cursor, limit)`, `admin_retry(conn, actor, RetryScope)` (audited with the count and at most 200 ids, as the CLI does). Routes: `GET /admin/mail/status`, `GET /admin/mail/messages?state=&plugin=&kind=&cursor=&limit=`, `POST /admin/mail/retry {scope}`. No response contains an address.

- [ ] **Step 1: Write the failing tests:** `an_admin_app_session_is_refused_the_mail_routes`; `the_journal_lists_ids_plugin_kind_and_state_and_no_address` (the body contains none of the seeded addresses); `retry_requeues_dead_messages_and_audits_the_count`; `status_reports_pending_sent_and_dead_in_24_hours`; paging by cursor neither skips nor repeats.
- [ ] **Step 2: Run** `cargo test -p server --test console_mail`. Expected: FAIL.
- [ ] **Step 3: Implement**, regenerate OpenAPI and types, prepare `.sqlx`.
- [ ] **Step 4: Run** the new test and `cargo test -p mail`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(mail): read and retry the mail queue from the console (ADR-0095)`.

### Task 3: The console shell, host selection and sign-in

**Files:**
- Create: `web/src/middleware.ts`, `web/src/app/console/layout.tsx`, `web/src/app/console/login/page.tsx`, `web/src/app/console/change-password/page.tsx`, `web/src/app/console/page.tsx` (redirects to People), `web/src/components/console/{ConsoleShell,ConsoleLogin}.tsx` and their CSS module, `web/src/lib/api/hooks/console.ts` (`useConsoleLogin`, `useConsoleLogout`), `web/src/mocks/handlers/console.ts`
- Modify: `web/src/lib/i18n/{en,uk}.ts`, `web/src/lib/session.ts` (the expired-session hook sends the console to its own sign-in), `web/scripts/build-desktop.mjs` (the export leaves `out/console` out and builds without the middleware)
- Test: `web/e2e/console-host.spec.ts`

**Interfaces:**
- Consumes: `clientConfig.public.server.admin_origin` (Task 0), `POST /console/login` and `POST /auth/logout` (plan 2).
- Produces: `middleware(request)`: when the request's host equals the host of `admin_origin`, `/` and any path outside `/console`, `/api`, `/_next`, `/brand` and static files redirect to `/console`; on any other host `/console/**` is rewritten to the not-found page. The shell shows the sidebar sections of Tasks 4–7, the signed-in name, "Sign out" and, for a student-app session (a `GET /me` that works only with the app cookie, which the console never has), nothing: the console asks only its own cookie, so a missing console cookie is the sign-in page.

- [ ] **Step 1: Write the failing e2e tests** (Playwright, mock API, two hosts via `extraHTTPHeaders` host override or two base URLs `localhost` / `127.0.0.1`):
  - `the_admin_host_serves_only_the_console` — `/`, `/courses`, `/calendar` redirect to `/console/login` when signed out; `/api/v1/health` answers.
  - `the_app_host_has_no_console` — `/console`, `/console/login` are the not-found page.
  - `a_console_sign_in_reaches_people_and_sign_out_returns_to_the_sign_in`.
  - `a_401_on_a_console_call_goes_to_the_console_sign_in_not_the_students`.
  - `a_temporary_password_sends_the_administrator_to_change_it_first`.
- [ ] **Step 2: Run** `npx playwright test e2e/console-host.spec.ts`. Expected: FAIL.
- [ ] **Step 3: Implement** the Interfaces block; MSW handlers for `/console/login` (cookie-less in the mock: a module flag) and `/me`.
- [ ] **Step 4: Run** the spec, `npm run typecheck`, `npm run build`, `npm run build:desktop` and check `web/out` has no `console` directory. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(web): the console shell, its sign-in and host selection (ADR-0095)`.

### Task 4: The existing administration pages move into the console

**Files:**
- Move: `web/src/app/(app)/admin/{people,groups,institution,audit}/page.tsx` to `web/src/app/console/<same>/page.tsx`; `web/src/components/admin/*` to `web/src/components/console/`; the terms half of the old admin courses page becomes `console/terms/page.tsx`
- Delete: `web/src/app/(app)/admin/**`, `AdminCourses` remains only as `Terms`
- Modify: `web/src/lib/nav.ts` (the administrator's `nav.admin` item becomes an external link to `admin_origin`, label "Admin console"), `web/e2e/routes.ts`, `web/e2e/admin-courses.spec.ts` (reads the all-courses tab only), `web/src/mocks/handlers/admin.ts`, i18n keys renamed once
- Test: `web/e2e/console-pages.spec.ts`, existing smoke specs

- [ ] **Step 1: Write the failing tests:** `every_moved_page_renders_in_the_console_both_themes` (the smoke list for the admin role moves to `/console/*`); `the_student_app_has_no_admin_routes` (`/admin`, `/admin/people` are not found on the app host); `an_administrator_sees_an_open_the_admin_console_link_and_no_admin_section`.
- [ ] **Step 2: Run** the specs. Expected: FAIL.
- [ ] **Step 3: Implement** the moves with `git mv` (history), fix imports, remove dead i18n keys in both catalogues.
- [ ] **Step 4: Run** `npm run typecheck`, `npm run build`, `npm run lint` (if it can run), the specs. Expected: PASS.
- [ ] **Step 5: Commit** — `refactor(web): the administration pages are the console's (ADR-0095)`.

### Task 5: Plugins page

**Files:**
- Create: `web/src/app/console/plugins/page.tsx`, `web/src/app/console/plugins/[id]/page.tsx`, `web/src/components/console/{ConsolePlugins,PluginDetail,SettingField}.tsx` + CSS, `web/src/lib/api/hooks/plugins-admin.ts`
- Modify: mocks, i18n
- Test: `web/e2e/console-plugins.spec.ts`

**Interfaces:** consumes Task 1's routes. `SettingField` renders one input from a setting's `kind` and `enum`/`items` (text, number, switch, list, pick), a write-only field for a secret ("set" / "not set", never the value), the description as help text, and the default; a wrong value shows the server's field message.

- [ ] **Step 1: Write the failing tests:** `the_list_shows_each_plugin_with_status_and_what_still_needs_setup`; `a_setting_form_is_built_from_the_manifest_spec` (each kind of the mock plugin gets its input); `a_secret_is_set_but_never_shown`; `a_refused_value_is_explained_and_nothing_is_saved`; `disabling_asks_first_and_says_queued_mail_is_cancelled`; `the_call_log_shows_counts_and_the_newest_calls`.
- [ ] **Step 2: Run** the spec. Expected: FAIL.
- [ ] **Step 3: Implement.** A plugin that `missing_settings` would warn about is marked "needs setup" in the list, as `doctor` names it.
- [ ] **Step 4: Run** the spec and `npm run typecheck`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(web): plugins in the console (ADR-0095)`.

### Task 6: Units and roles page

**Files:**
- Create: `web/src/app/console/units/page.tsx`, `web/src/components/console/{ConsoleUnits,UnitRoles,RoleAbilities}.tsx` + CSS, hooks in `web/src/lib/api/hooks/units.ts` if absent
- Test: `web/e2e/console-units.spec.ts`

**Interfaces:** consumes the existing `/units`, `/units/{id}`, `/units/{id}/roles`, `/units/{id}/roles/{user_id}`, `/units/{id}/candidates`, `/admin/unit-roles`. Layout: the tree on the left, the selected unit's roles on the right (add a person through the candidates search, `manager` or `viewer`, remove), and above the tree two columns of switches, one per ability, for `manager` and `viewer`.

- [ ] **Step 1: Write the failing tests:** `an_admin_builds_a_unit_tree_and_renames_and_moves_a_unit`; `roles_are_added_by_search_and_removed`; `an_ability_switch_changes_the_role_for_every_unit_and_says_so`; `a_name_taken_under_the_parent_is_explained`; `deleting_a_unit_with_courses_is_refused_with_the_reason`.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the spec and typecheck. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(web): units and unit roles in the console (ADR-0095)`.

### Task 7: Mail page

**Files:**
- Create: `web/src/app/console/mail/page.tsx`, `web/src/components/console/ConsoleMail.tsx` + CSS, hooks `web/src/lib/api/hooks/mail.ts`
- Test: `web/e2e/console-mail.spec.ts`

- [ ] **Step 1: Write the failing tests:** `the_mail_page_shows_the_queue_counts_and_the_journal`; `a_filter_by_state_and_plugin_narrows_the_journal`; `retry_dead_messages_asks_how_many_and_reports_how_many_were_put_back`; `no_address_appears_on_the_page`; `without_mail_configured_the_page_says_so`.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the spec and typecheck. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(web): the mail queue in the console (ADR-0095)`.

### Task 8: Deployment, installer and release notes

**Files:**
- Modify: `deploy/Caddyfile` (a second site block `{$VUMLO_ADMIN_DOMAIN}`: `/api/*` to the core with `header_up X-Real-IP`, the rest to the web upstream, same security headers, `Host` kept), `deploy/compose.yaml` and `compose.yaml` (publish `3001:3000` beside `3000:3000`), `docs/install.md` (the walk-through: DNS for the admin name, the Caddy variable, first sign-in, `admin set-password`), `docs/development.md` (the `127.0.0.1` / `localhost` dev story), `CHANGELOG.md`, `docs/decisions/in-progress.md` (ADR-0095 moves to "built"; the move to `done.md` is the owner's), `RELEASING.md` if the upgrade notes are listed there
- In the `create-vumlo` repository: `index.mjs` (`--admin-origin`; default `http://localhost:3001` for `http://localhost:3000`, else `admin.` in front of the host; `patchToml` writes `server.admin_origin`; `update` asks once for an installation whose `vumlo.toml` lacks it and never writes it without an answer; the final "Open ..." note names both addresses), `test.mjs`
- Test: `create-vumlo/test.mjs`, a Caddy validation, the full workspace

- [ ] **Step 1: Write the failing tests in `create-vumlo/test.mjs`:** `patch_toml_writes_the_admin_origin`; `the_admin_origin_defaults_from_the_public_one` (localhost and a public host); `update_asks_for_a_missing_admin_origin_and_writes_nothing_without_an_answer`; `an_admin_origin_equal_to_the_public_one_is_refused`; `the_summary_names_the_console_address`.
- [ ] **Step 2: Run** `node --test test.mjs` there. Expected: FAIL.
- [ ] **Step 3: Implement** the files above; validate the Caddyfile with `caddy validate` if Caddy is installed, else say so in the ledger.
- [ ] **Step 4: Run** the create-vumlo tests, then the **full definition of done** (Global Constraints) and the whole Playwright set, reading each exit code. Expected: all pass.
- [ ] **Step 5: Commit** per repository — `feat(deploy): the console's address and block (ADR-0095)`; `feat: ask for the admin address`.

---

## Left to the builder

- The exact widgets and layout of the pages, within the design tokens and the existing components (`Dialog`, `ConfirmDialog`, `PasswordField`).
- The development story: with `pnpm dev` the console is `http://127.0.0.1:3000` and the student app `http://localhost:3000` (set `server.admin_origin` and `VUMLO_API_ORIGIN` accordingly); with `pnpm dev:mock` the mock treats the host the same way.
- Whether `GET /admin/courses` keeps its path (deferred from plan 1); renaming it is an API change the owner has not asked for.
- Which institution settings of `vumlo.toml` the Institution page shows read-only, with a pointer to create-vumlo.

## Not in this plan

- A second factor, an email reset for administrators, custom unit roles, course-scope plugin settings, installing or removing plugins from the console, and running `doctor` in the browser (the spec's non-goals).
