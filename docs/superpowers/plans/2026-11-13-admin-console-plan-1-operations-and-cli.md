# Admin console, plan 1: operations layer, CLI wizard, course link

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An administrator configures plugin settings and unit-role abilities from the terminal with guided prompts, `doctor` says what still needs setup, and an admin finds every course on the normal courses page (the admin course page keeps only terms).

**Architecture:** Parsing and storing a setting from text moves into the `plugins` crate (one function the CLI wizard, the old `settings set`, and the later console route share). The abilities of a unit role get an actor-free inner function in `org` next to the actor-checked service function. The CLI adds `plugin configure`, `roles show|set` and a `doctor` hint. Audit rows keep source `cli`.

**Tech Stack:** Rust (axum, sqlx, clap), existing `vumlo-cli` integration-test harness (`tests/common`), Next.js and Playwright for the link.

**Spec:** `docs/superpowers/specs/2026-11-13-admin-console-design.md` (PR 0 and PR 1 of section 6). Plans for PR 2 (audience, console login) and PR 3 (console web surface) follow after this one lands.

## Amendments to the spec found while planning

- `plugins` already exposes actor-free `set_setting` / `unset_setting` / `list_settings`; only the text parsing sits in the CLI. So the "operations layer" for plugins is one new function, `set_setting_text`.
- The CLI gets `roles show|set` only. Unit tree and unit-role assignment need actor-based visibility logic and are built as the console page in plan 3, not as CLI commands.
- `plugin configure` reads answers from standard input line by line (as `plugin install` does), so it is scriptable and testable; it is not terminal-only. A secret is read hidden (`rpassword`) only when stdin is a terminal.

## Global Constraints

- Work on a branch, never `main`; small Conventional Commits (`feat(plugins): ...`); push only when asked. Branch `feat/admin-console` from the current branch `feat/org-units`.
- A CLI command that changes the database writes an audit row with source `cli` (`db::insert_cli_audit`); a command that changes nothing says so and writes nothing; `--dry-run` rolls back.
- Never print or audit a setting value that is secret; audit meta carries ids and key only (existing `audit_meta`).
- No new external crate (ADR-0014). `org` is added to `vumlo-cli` as a workspace path dependency only.
- No `unwrap`/`expect`/`panic!` outside tests; `anyhow` allowed in CLI code.
- Unit tests live in `foo/tests.rs`, never inline; CLI behaviour tests are integration tests in `backend/crates/vumlo-cli/tests/`.
- Definition of done (CLAUDE.md 14), from `backend/`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace` (run long tests one at a time, `CARGO_BUILD_JOBS=2`); `cargo sqlx prepare --workspace --check -- --all-targets` (run `cargo sqlx prepare --workspace -- --all-targets` after any query change); `SQLX_OFFLINE=true cargo check --workspace --all-targets`. From `web/`: `npm run typecheck`, `npm run lint`, `npm run build`.

## Review Focus

1. `plugin configure` input ends midway (EOF): nothing is committed and the exit code is non-zero. (Task 2)
2. A secret typed into `configure` appears nowhere: not in stdout, audit meta, or the outbox payload. (Task 2)
3. Windows line endings (`3\r\n`) in piped answers behave like `3`. (Task 2)
4. `roles set viewer --abilities bogus` changes nothing and names the valid abilities; `--abilities none` sets the empty set. (Task 4)
5. `doctor` does not warn about a per-course setting that has no default, only about institution-scope ones. (Task 3)
6. A search containing `%` or `_` matches those characters literally and does not list everything. (Task 0)
7. A filter combined with a cursor never skips or repeats a course across pages. (Task 0)

## File structure

- Task 0 (course browsing for admins): `access` (`Action::ViewAllCourses`), `teaching` admin service and route, `web/src/app/(app)/courses/page.tsx`, new `web/src/components/courses/AllCourses.tsx`, `AdminCourses.tsx` reduced to terms, new `web/e2e/admin-courses.spec.ts`.
- Modify `backend/crates/plugins/src/service.rs`, `lib.rs` (export), test in `backend/crates/plugins/src/service/tests.rs`: `set_setting_text`, `missing_settings`.
- Modify `backend/crates/vumlo-cli/src/plugin_cmd.rs`: `settings_set` uses `set_setting_text`; new `configure`.
- Create `backend/crates/vumlo-cli/src/roles_cmd.rs`: `show`, `set`.
- Modify `backend/crates/vumlo-cli/src/main.rs` (commands), `src/doctor.rs` (hint), `Cargo.toml` (`org`).
- Modify `backend/crates/org/src/service.rs`, `lib.rs`: `role_abilities`, `apply_role_abilities`.
- Tests: `backend/crates/vumlo-cli/tests/plugins.rs`, new `tests/roles.rs`, `tests/doctor.rs`.
- Modify `docs/decisions/in-progress.md` (ADR line), `docs/install.md` or the CLI reference where `plugin settings` is documented.

---

### Task 0: Admin browses every course on `/courses`; the admin course page keeps only terms

The admin's own `/courses` page gets an "All courses" tab (every course with owner, term, student count, each row a link to the course), filtered and paged on the server so it works with thousands of courses. The table and the "Change owner" dialog leave `/admin/courses`, which keeps the terms. Changing the owner stays where it already is: the owner section of course settings (`useTransferOwner`, `PUT /courses/{id}/owner`).

**Files:**
- Modify: `backend/crates/access/src/lib.rs` (new `Action::ViewAllCourses`, allowed for `is_admin`, listed with the other course-oversight actions), `backend/crates/api-types/src/lib.rs` (`AllCoursesQuery`), `backend/crates/learning/teaching/src/repo/admin.rs`, `service/admin.rs` and `routes/admin.rs` (`admin_courses` / `list_admin_courses` take the filters and authorize `ViewAllCourses` instead of `ViewAdminReports`), `backend/crates/server/tests/authorization.rs`, `web/openapi.json` and `web/src/lib/api/schema.d.ts` (regenerate with `npm run gen:api`), `backend/.sqlx/` (`cargo sqlx prepare --workspace -- --all-targets`).
- Modify: `web/src/app/(app)/courses/page.tsx` (admin sees "My courses" and "All courses"), `web/src/components/admin/AdminCourses.tsx` (terms only: delete the courses table, the filters, `useChangeOwner` use and the owner dialog), `web/src/lib/api/hooks/admin.ts` (delete `useChangeOwner`; `useAdminCourses` becomes the paged `useAllCourses(filter)`), `web/src/mocks/handlers/admin.ts` (honours the filters, `limit` and `cursor`), i18n catalogues (en, uk) for the new tab labels and "Show more".
- Create: `web/src/components/courses/AllCourses.tsx` (the table moved from `AdminCourses`; title cell is `<Link href={courseHref(c.course_id)}>`).
- Test: `backend/crates/learning/teaching/src/service/tests.rs`, `web/e2e/admin-courses.spec.ts`.

**Interfaces:**
- Consumes: `Action` match in `access::authorize`; `common::search::like_pattern(text: &str) -> String` (escapes `%` and `_`); `Page::from_rows(rows, limit, |row| row.course_id)`; `PageParams { after: Option<Uuid>, limit: u32 }`; `useTransferOwner(courseId)`; `courseHref(id: string, tab?: string): string` from `@/lib/routes`; the `ListUsersQuery` shape in `api-types` as the pattern for a filter plus cursor.
- Produces:
  - `Action::ViewAllCourses` (course oversight: stays available to an admin's app session in plan 2; not an administration action).
  - `AllCoursesQuery { term_id: Option<Uuid>, status: Option<CourseStatus>, q: Option<String> (1 to 100 characters, garde-validated), cursor: Option<String>, limit: Option<u32> }`.
  - `repo::admin_courses(conn, filter: &AdminCourseFilter, after: Option<Uuid>, limit: i64)` where `AdminCourseFilter { term_id: Option<Uuid>, status: Option<CourseStatus>, q: Option<String> }` (the `q` already passed through `like_pattern`): regular courses, **newest first** (`ORDER BY c.id DESC`, cursor `c.id < after`; ids are UUID v7), with `q` a case-insensitive substring of the code, the title or the owner's name or surname (`ponytail:` comment: sequential scan, add a trigram index if an installation reaches hundreds of thousands of courses).
  - `GET /admin/courses` keeps its path and response shape; renaming it to `GET /courses/all` is the builder's call if it is cheap.
  - `useAllCourses(filter: { termId?: string; status?: CourseStatus; q?: string })`: first page by query key from the filter, `loadMore()` appends the next page with the cursor, `hasMore: boolean`; changing the filter starts again from the first page.

- [ ] **Step 1: Create the branch** `git switch -c feat/admin-console` (from `feat/org-units`).
- [ ] **Step 2: Write failing backend tests** in `teaching/src/service/tests.rs` (create courses in two terms, two statuses, with different owners): `admin_courses` returns every regular course for an admin actor and the authorization error for a teacher and for a student (now under `ViewAllCourses`); `term_id` and `status` each narrow the list; `q` matches code, title, owner name and owner surname, ignoring case; `q = "50%"` matches a title containing the literal `50%` and not everything (Review Focus 6); three pages of limit 2 over five matching courses, newest first, have no gap and no duplicate when a filter is set (Review Focus 7); a request with an empty `q` is rejected as `422` (route-level test in `teaching`'s route tests or `server/tests`).
- [ ] **Step 3: Run** `cargo test -p teaching admin_courses` → FAIL. Add `Action::ViewAllCourses`, `AllCoursesQuery`, `AdminCourseFilter`, the filtered DESC query and the service/route changes; update the `authorization.rs` entry; run `cargo sqlx prepare --workspace -- --all-targets`; `cargo test -p teaching admin_courses` and `cargo test -p server --test authorization` → PASS.
- [ ] **Step 4: Write the failing e2e** in `web/e2e/admin-courses.spec.ts` (sign in with `signIn(page, accounts.admin)` as `downloads.spec.ts` does): on `/courses` the "All courses" tab lists a seed course owned by a teacher; typing part of the owner's surname in the search box narrows the list to that teacher's courses; the term and status selects narrow it too; clicking a title opens that course; `/admin/courses` shows the terms and no courses table; in that course's settings the admin changes the owner (the existing owner section) and the new owner's name shows. Run `cd web && npx playwright test e2e/admin-courses.spec.ts` → FAIL. (The "Show more" button is covered by the backend paging test; the mock seed is not enlarged for it.)
- [ ] **Step 5: Implement the web change** as listed in Files: the search box waits about 300 ms after typing before it asks; the term select defaults to the current term and has "All terms"; the status select is All, Active (published), Draft, Archived; "Show more" is shown while `hasMore`. Delete what became unused (`useChangeOwner`, the owner dialog, its i18n keys and mock branch) and regenerate the API types and OpenAPI.
- [ ] **Step 6: Run** the e2e, `npm run typecheck`, `npm run lint`, `npm run build`, and `cargo sqlx prepare --workspace --check -- --all-targets` → PASS.
- [ ] **Step 7: Commit** `feat(web): admins browse every course on the courses page`.

---

### Task 1: `set_setting_text` in `plugins`

**Files:**
- Modify: `backend/crates/plugins/src/service.rs` (after `set_setting`), `backend/crates/plugins/src/lib.rs` (re-export)
- Modify: `backend/crates/vumlo-cli/src/plugin_cmd.rs` (`settings_set`)
- Test: `backend/crates/plugins/src/service/tests.rs`

**Interfaces:**
- Consumes: `set_setting`, `spec_in_scope`, `SettingSpec::parse_text`, `PluginsError::BadSetting { key, why }`.
- Produces: `pub async fn set_setting_text(conn: &mut PgConnection, key: &SecretKey, plugin_id: &str, name: &str, text: &str, course: Option<Uuid>) -> Result<bool, PluginsError>`: parses `text` with the setting's spec (`BadSetting` with the parse reason on failure), then stores as `set_setting` does; returns whether anything changed.

- [ ] **Step 1: Write failing tests** in `service/tests.rs`, using the existing install fixtures there: `set_setting_text(.., "limit", "3", None)` stores number `3` and returns `true`; the same call again returns `false`; `"abc"` for a number setting returns `PluginsError::BadSetting { key: "limit", .. }` and stores nothing; a secret setting given `"hunter2"` returns `true` and `list_settings` shows `value == None, is_set == true`.
- [ ] **Step 2: Run** `cargo test -p plugins set_setting_text` → FAIL (function missing).
- [ ] **Step 3: Implement** by resolving the spec with `spec_in_scope`, mapping `parse_text`'s `Err(why)` to `BadSetting`, then calling `set_setting`. Export from `lib.rs` next to `set_setting`.
- [ ] **Step 4: Switch `settings_set`** in `plugin_cmd.rs` to call `plugins::set_setting_text` instead of `get_setting` + `parse_text` + `set_setting`; keep its messages, audit, dry-run and the "has this value already" branch.
- [ ] **Step 5: Run** `cargo test -p plugins` and `cargo test -p vumlo-cli --test plugins` (one at a time). Expected: PASS, including the existing settings tests unchanged.
- [ ] **Step 6: Commit** `refactor(plugins): set a setting from text in the plugins crate`.

---

### Task 2: `vumlo-cli plugin configure <id>`

**Files:**
- Modify: `backend/crates/vumlo-cli/src/main.rs` (`PluginCommand::Configure { plugin_id: String, dry_run: bool }`, dispatch), `src/plugin_cmd.rs`
- Test: `backend/crates/vumlo-cli/tests/plugins.rs`

**Interfaces:**
- Consumes: `plugins::list_settings(conn, plugin_id, None) -> Vec<SettingView>`, `plugins::set_setting_text` (Task 1), `state(&SettingView)` and `kind_of(&SettingView)` (existing in `plugin_cmd.rs`), `audit_meta`, `db::insert_cli_audit`, `rpassword::prompt_password`, `std::io::IsTerminal`.
- Produces: `pub async fn configure(options: &Options, plugin_id: &str, dry_run: bool) -> anyhow::Result<()>`.

Behaviour: for each institution-scope setting (the order `list_settings` returns), print `key  (kind)`, the description, `current: <state>`, then read one answer line from stdin, trailing `\r\n` removed. An empty line keeps the current value. A secret setting is read with `rpassword` when stdin is a terminal and as a plain line otherwise. An answer the spec rejects prints the reason and asks the same setting again. End of input before every setting is answered is an error: nothing is committed. All changes happen in one transaction, one `plugin.setting.set` audit row per changed key (same meta as `settings set`), committed at the end; `--dry-run` rolls back and prints "would set ...". A plugin with no institution-scope settings prints `<id> declares no institution settings` and exits 0; per-course settings are named in one closing line ("N are set per course").

- [ ] **Step 1: Write failing tests** in `tests/plugins.rs`, with a manifest declaring `limit` (number, default 10), `unit` (enum), `api_token` (secret string, no default) and one course-scope setting. Tests: (a) stdin `"5\ncharacters\ntok-secret\n"` sets all three, three audit rows, outbox has one `plugin.settings.changed` per changed key batch; (b) stdin `"\n\n\n"` changes nothing and writes no audit row; (c) stdin `"abc\n5\n\n\n"` shows the rejection text then accepts `5`; (d) stdin `"5\n"` only (EOF) exits non-zero and `plugins.settings` is unchanged (Review Focus 1); (e) `"5\r\n\r\n\r\n"` behaves like (a)'s first answer (Review Focus 3); (f) the secret appears in neither stdout nor audit meta nor outbox payload (Review Focus 2); (g) a plugin with only course-scope settings prints the "declares no institution settings" line.
- [ ] **Step 2: Run** `cargo test -p vumlo-cli --test plugins configure` → FAIL (unknown subcommand).
- [ ] **Step 3: Implement** `configure` and the clap variant with the doc comment "Walk through the settings of a plugin one by one." Reuse `ready`, `key` helpers already in `plugin_cmd.rs`.
- [ ] **Step 4: Run** `cargo test -p vumlo-cli --test plugins` → PASS.
- [ ] **Step 5: Commit** `feat(vumlo-cli): plugin configure walks through the settings`.

---

### Task 3: `doctor` names plugins that need setup

**Files:**
- Modify: `backend/crates/plugins/src/service.rs` (`missing_settings`), `lib.rs`; `backend/crates/vumlo-cli/src/doctor.rs` (`plugins` check)
- Test: `backend/crates/vumlo-cli/tests/doctor.rs`, `backend/crates/plugins/src/service/tests.rs`

**Interfaces:**
- Consumes: `plugins::list`, `list_settings`.
- Produces: `pub async fn missing_settings(conn: &mut PgConnection) -> Result<Vec<(String, Vec<String>)>, PluginsError>`: for each install, the keys of institution-scope settings that have no stored value and no default (`!is_set && spec.default.is_none()`), skipping installs with none.

- [ ] **Step 1: Write failing tests.** In `service/tests.rs`: an install with `api_token` (no default, unset), `limit` (default 10) and a course-scope setting without default returns `[("hello", ["api_token"])]`; after `set_setting_text` for `api_token` it returns `[]` (Review Focus 5). In `tests/doctor.rs`: with that install the `plugins` check is a warning containing `plugin configure hello`; with the token set it passes.
- [ ] **Step 2: Run both** → FAIL.
- [ ] **Step 3: Implement** `missing_settings`, and in `doctor::plugins` return `Warn` with text `hello needs setup: api_token (run `vumlo-cli plugin configure hello`)` when the list is non-empty and no other warning outranks it (keep the existing warnings first).
- [ ] **Step 4: Run** `cargo test -p plugins missing_settings` and `cargo test -p vumlo-cli --test doctor` → PASS.
- [ ] **Step 5: Commit** `feat(vumlo-cli): doctor names plugins that still need settings`.

---

### Task 4: `vumlo-cli roles show | set`

**Files:**
- Modify: `backend/crates/org/src/service.rs` (split the two ability functions), `org/src/lib.rs`, `backend/crates/vumlo-cli/Cargo.toml` (`org.workspace = true`), `src/main.rs` (`Command::Roles(RolesCommand)`)
- Create: `backend/crates/vumlo-cli/src/roles_cmd.rs`
- Test: `backend/crates/vumlo-cli/tests/roles.rs`, `backend/crates/org/src/tests.rs`

**Interfaces:**
- Consumes: `UnitRole` and `UnitAbility` with their text forms (`UnitAbility::ALL`, `as_str`, and the parse the `text_enum!` macro generates; verify its name), `repo::list_role_abilities`, `repo::lock_role_abilities`, `repo::replace_role_abilities`, `db::insert_cli_audit`.
- Produces: `pub async fn role_abilities(conn: &mut PgConnection) -> Result<Vec<(UnitRole, Vec<UnitAbility>)>, OrgError>` (no actor) and `pub async fn apply_role_abilities(conn: &mut PgConnection, role: UnitRole, abilities: Vec<UnitAbility>) -> Result<Vec<&'static str>, OrgError>` (sorts, de-duplicates, locks, replaces, returns the stored names). `unit_role_abilities` and `set_unit_role_abilities` keep their signatures: authorize, call the inner function, and (for set) audit as today with the returned names.

- [ ] **Step 1: Write failing tests.** `org/src/tests.rs`: `apply_role_abilities` sorts and de-duplicates (`[ManageRoster, ViewGrades, ViewGrades]` stores `["view_grades","manage_roster"]` in `UnitAbility::ALL` order); the actor-checked `set_unit_role_abilities` still refuses a non-admin and still writes its audit row (existing tests must stay green). `tests/roles.rs`: `roles show` prints each role with its abilities and shows `view` as always on; `roles set viewer --abilities view_grades,manage_roster` changes the stored set and writes one audit row `unit.abilities.set` with source `cli` and meta `{role, abilities}`; running it again says nothing was changed and writes no row; `--abilities bogus` exits non-zero, names the valid abilities, changes nothing (Review Focus 4); `--abilities none` stores the empty set; `--dry-run` changes nothing.
- [ ] **Step 2: Run** `cargo test -p org abilities` and `cargo test -p vumlo-cli --test roles` → FAIL.
- [ ] **Step 3: Implement the org split** and export the two new functions; add the `org` dependency to `vumlo-cli`; implement `roles_cmd::{show, set}` and the clap enum (`RolesCommand::Show`, `RolesCommand::Set { role, abilities, dry_run }`). `set` compares with the current set first to print "nothing was changed".
- [ ] **Step 4: Run** `cargo test -p org`, `cargo test -p vumlo-cli --test roles`, and the architecture test `cargo test -p server --test architecture` (the CLI now names `org`; if it objects, the allowed list in that test is the place to say so). Expected: PASS.
- [ ] **Step 5: Commit** `feat(vumlo-cli): roles show and set the abilities of the unit roles`.

---

### Task 5: Documentation and the definition of done

**Files:**
- Modify: `docs/decisions/in-progress.md` (one ADR line for plan 1: `plugin configure`, `roles`, `doctor` hint, `set_setting_text`), the page that documents `plugin settings` (`docs/install.md` or the CLI reference), the CLI doc comments already written in `main.rs`.

- [ ] **Step 1: Add the ADR line and the doc entries** for `plugin configure`, `roles show|set`, and the doctor message.
- [ ] **Step 2: Run the full definition of done** from `backend/` (every command in Global Constraints, tests one at a time) and the web checks from `web/`. Expected: all pass.
- [ ] **Step 3: Commit** `docs: plugin configure, roles and the doctor hint`.

## Self-review

- Spec coverage for PR 0 and PR 1: all courses on `/courses` for admins, with the duplicate list and owner dialog removed from the admin page (Task 0; this replaces the spec's "rows link to the course", see its section 5.5), `plugins` set-from-text (1), `plugin configure` (2), `doctor` hint (3), `roles show|set` (4), docs and ADR (5). The spec's CLI `units` commands are deliberately dropped (see Amendments) and move to plan 3.
- Types agree across tasks: `set_setting_text` (Task 1) is the function Task 2 calls; `missing_settings` (Task 3) uses `list_settings` and `set_setting_text`; `apply_role_abilities` (Task 4) returns the names both callers audit.
- Proportion: five code tasks, test-first, no function bodies except behaviour the tests do not pin.
