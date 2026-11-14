# Units in the web app and the activity type contract: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** finish roadmap item 2 (org units in the people's app, ADR-0094 PR B) and build item 3 (the activity type contract
that replaces `BuiltinType`, ADR-0097), in one PR, one branch `feat/units-web-activity-types`.

**Architecture:** Part A is web only: units hooks and components move to `web/src` (the console already imports from
there), course screens read the server's `can`, a `/units` page and a unit picker are added, the mock world learns units.
Part B is backend only: a new crate `activity` holds the contract, the document model it names and a registry filled by the
composition root; the five built-in types implement it in `content`; every decision that names a type asks the registry.
The two parts touch no common file, so each task lands on its own.

**Tech Stack:** Rust (sqlx, axum), Postgres, Next.js 16 + React 19 (`web/`), Vite + React (`web-admin/`), MSW mocks,
Playwright.

**Specs:** `docs/superpowers/specs/2026-11-13-org-units-design.md` (sections 5b, 8, 10) and
`docs/superpowers/specs/2026-11-14-activity-types-design.md`.

## Global Constraints

- The console (`web-admin/`) keeps every system-wide administration setting; nothing leaves it. The people's app gets only
  what a dean, a viewer, a teacher or a student does, bounded by the grants the server returns.
- The unit picker lists the units of `GET /units` whose `abilities` hold `manage_courses`. An admin without a unit role
  places courses from the console. No backend change for item 2.
- No behaviour change from item 3: the same items are done, the same lists come back, no API shape changes, `type` stays a
  string. No migration.
- No compatibility layer (CLAUDE.md section 13): when a name moves, its callers change; no re-export left behind.
- Unit tests live in `tests.rs` beside the file (CLAUDE.md section 12); DB tests use `db_test`.
- Run cargo with `CARGO_BUILD_JOBS=2`, one command at a time. After any `query!` change:
  `cargo sqlx prepare --workspace -- --all-targets --all-features` with Postgres up
  (`docker compose up -d postgres`; `DATABASE_URL=postgres://vumlo:vumlo@localhost:5432/vumlo`). Never run it on a broken
  build (it wipes `backend/.sqlx`; restore with `git checkout backend/.sqlx`).
- Check each Rust task with `cargo check --workspace --all-targets --all-features`, and a new crate alone with
  `cargo test -p activity --no-run`.
- Edit files with Edit/Write. Revert `web/pnpm-lock.yaml` churn. Never commit `graphify-out/`. Conventional Commits,
  small, no push (the owner pushes).
- Every new API operation is already in `server/tests/authorization.rs` (PR A); this PR adds none.

## Review Focus

- A dean (unit manager) who holds no course role opens a course of their unit and sees the staff screens their abilities
  allow, and none that they do not (a viewer must not see an edit control).
- A student never sees the unit picker, the units link or `can`-gated staff controls, whatever `GET /units` returns.
- A `no_submission` assignment is not done until its score is published; a quiz is done at submit, before it is graded.
- A row whose type is not registered fails with a clear error and no panic; `install` twice and a forgotten `install` are
  errors.
- Moving a course to "No unit" and back works, and a course in a unit the person may not place into keeps showing its
  unit without letting them pick it.

---

## Part 0: the repository process

### Task 0: ADR line and branch bookkeeping

**Files:**
- Modify: `docs/decisions/in-progress.md` (add the ADR-0097 line), `docs/superpowers/specs/2026-11-13-org-units-design.md`
  (section 8: the picker rule; status line)

- [ ] **Step 1:** Add `ADR-0097 - Activity types: ...` to `in-progress.md`, one line, in the style of its neighbours: the
  contract, the registry, what is built (nothing yet), what is missing (everything, filled in Task 15), the decision list of
  spec section 9. In the org-units spec section 8 replace "admin, or `manage_courses` somewhere" by "some `GET /units` item
  holds `manage_courses`; an admin places courses from the console".
- [ ] **Step 2: Commit** `docs: ADR-0097 activity types, the picker rule of ADR-0094`.

---

## Part A: org units in the people's app

### Task 1: Unit hooks move to `web/` and the course hooks learn `can` and `unit_id`

**Files:**
- Create: `web/src/lib/api/hooks/units.ts` (moved from `web-admin/src/hooks/units.ts`)
- Modify: `web-admin/src/**` imports of `@console/hooks/units` -> `@/lib/api/hooks/units`; `web/src/lib/api/hooks/courses.ts`
- Delete: `web-admin/src/hooks/units.ts`

**Interfaces:**
- Produces (new, in `units.ts`): `useUnitCourses(id: string | null)` -> `Query<Course[]>` (all pages of
  `/units/{id}/courses`, key `"/units/{id}/courses"`); `useSetCourseUnit(courseId: string)` -> mutation
  `run(unitId: string | null)` that calls `PUT /courses/{id}/unit` `{unit_id}` and invalidates `/courses`, `/units`.
- Produces (changed, `courses.ts`): `useCourse(id)` returns `CourseDetail` (has `can: CourseCan`, `unit_id`);
  `useCreateCourse().run(title, termId?, unitId?)`; `useCreateFromTemplate().run(templateId, title, unitId?)`.
- Consumes: `fetchAll`, `api`, `useQuery`, `useMutation`, `invalidate` as the file already does.

- [ ] **Step 1:** Move the file with its existing hooks unchanged; fix every import in `web-admin/src` (grep
  `hooks/units`). Add the two hooks.
- [ ] **Step 2:** Change `useCourse` to the `CourseDetail` type; add the optional unit argument and send `unit_id` only when
  set (the request field is optional).
- [ ] **Step 3: Verify** `cd web && pnpm typecheck` and `cd web-admin && pnpm typecheck && pnpm lint` pass.
- [ ] **Step 4: Commit** `refactor(web): the unit hooks live in the app both web projects share`.

### Task 2: Shared unit components

**Files:**
- Create: `web/src/components/units/tree.ts` (`rows`), `UnitDialogs.tsx`, `UnitRoles.tsx`, `Units.module.css`
  (the rules these use from `web-admin/src/components/Admin.module.css`)
- Modify: `web-admin/src/components/ConsoleUnits.tsx` (imports)
- Delete: `web-admin/src/components/UnitDialogs.tsx`, `UnitRoles.tsx`

**Interfaces:**
- Produces: `rows(all: UnitSummary[]): { unit: UnitSummary; level: number }[]` (depth first, by name);
  `NewUnit`, `RenameUnit`, `MoveUnit`, `UnitRoles` with the props they have today.
- Consumes: `@/lib/api/hooks/units`, `@/components/Dialog`.

- [ ] **Step 1:** Move the three files and the `rows` helper; copy only the CSS rules they use into `Units.module.css`
  (the console's `Admin.module.css` keeps its own). `RoleAbilities.tsx` stays where it is.
- [ ] **Step 2: Verify** typecheck of both projects, `cd web-admin && pnpm exec playwright test e2e/units.spec.ts` (the
  console's unit pages behave as before).
- [ ] **Step 3: Commit** `refactor(web): the unit tree, dialogs and roles are shared with the console`.

### Task 3: The mock world has units, grants and `can`

**Files:**
- Create: `web/src/mocks/handlers/units.ts` (from `web-admin/src/mocks/units.ts`)
- Modify: `web/src/mocks/{handlers.ts,seed.ts,db.ts,logic.ts,identities.ts}`, `web/src/mocks/handlers/core.ts`
  (`GET /courses/:id`), `web-admin/src/mocks/start.ts`
- Delete: `web-admin/src/mocks/units.ts`

**Interfaces:**
- Produces: `unitHandlers`; `logic.ts`: `unitGrantOf(user, courseId): Set<UnitAbility>` and `canOf(user, course): CourseCan`
  (staff of the course get every field true; a grant sets the fields its abilities map to, as in spec 5b of org units;
  anything else all false); an identity `accounts.dean` (a teacher who teaches nothing in the unit they manage) and
  `accounts.viewer`.
- `GET /units` returns, for a console admin, all units; for anyone else the units their grants cover with the union of
  abilities. `GET /units/:id/courses` pages real seed courses by `unit_id` and is 404 when the unit is not visible.
  `PUT /courses/:id/unit` needs `manage_courses` on both units and is 404/403 as the server is.

- [ ] **Step 1:** Seed `unit_id` on two courses and put one course in the dean's unit that the dean does not teach.
- [ ] **Step 2:** `canView`/`isStaff` consult `unitGrantOf`; `GET /courses/:id` returns `can` and `unit_id`.
- [ ] **Step 3:** Move the console's mock into `web/src` and make its handlers grant-aware; `start.ts` imports it from
  `@/mocks/handlers/units`.
- [ ] **Step 4: Verify** `cd web-admin && pnpm exec playwright test e2e/units.spec.ts` still passes; `cd web && pnpm
  typecheck`.
- [ ] **Step 5: Commit** `feat(web): units, grants and can in the mock world`.

### Task 4: Course screens read `can`

**Files:**
- Modify: `web/src/components/course/useCourseAccess.ts`; its callers: `app/(app)/course/{page,settings/page,grades/page}.tsx`,
  `components/course/{People,CourseHeader}.tsx`, `components/item/{ItemView,ItemFrame}.tsx`,
  `components/grading/GradeWorkspace.tsx`, `components/author/ItemEditor.tsx`
- Test: `web/e2e/units.spec.ts` (created in Task 7; this task checks with the existing e2e and typecheck)

**Interfaces:**
- Produces: `useCourseAccess(courseId)` returns `{ loading, can: CourseCan, staff: boolean }` where `can` is the server's
  and `staff` is `Object.values(can).some(Boolean)`. Gates: settings page and `CourseHeader` -> `can.manage_course`;
  `People` -> `can.manage_roster`; authoring in `course/page.tsx` and `ItemEditor` -> `can.edit_content`;
  `GradeWorkspace` -> `can.grade`; `course/grades/page.tsx` -> `can.view_grades`; `ItemView` and `ItemFrame` -> `staff`.
- A student's `can` is all false (the server says so), so students are unchanged. An owner or enrolled teacher has every
  field true.

- [ ] **Step 1:** Rewrite `useCourseAccess` on `useCourse(courseId).data?.can`; drop the guess from `/me/courses`.
- [ ] **Step 2:** Replace each caller's `staff` with the ability named above (read the file first: a screen that really
  means "any staff" keeps `staff`).
- [ ] **Step 3: Verify** `cd web && pnpm typecheck && pnpm lint && pnpm exec playwright test e2e/pages.spec.ts
  e2e/gradebook.spec.ts e2e/grading.spec.ts e2e/editor.spec.ts` (a teacher and a student still see what they saw).
- [ ] **Step 4: Commit** `refactor(web): course screens follow the server's can, not the role`.

### Task 5: The Units page, the navigation link and the i18n

**Files:**
- Create: `web/src/app/(app)/units/page.tsx`, `web/src/components/units/Units.tsx`
- Modify: `web/src/lib/nav.ts`, `web/src/components/shell/Shell.tsx`, `web/src/components/Icon.tsx`,
  `web/src/lib/i18n/{en,uk}.ts`

**Interfaces:**
- `navFor(role: Role, options: { units: boolean }): NavItem[]` appends `{ label: "nav.units", href: "/units", icon: "units",
  match: starts("/units") }` when `options.units`. `Shell` passes `(useUnits().data?.length ?? 0) > 0`.
- `Units`: left, the tree of `useUnits()` (`rows`); right, `useUnitCourses(selected)` as links to `courseHref`. On a unit
  whose `abilities` include `manage_units`: the `NewUnit`/`RenameUnit`/`MoveUnit` buttons and `UnitRoles`. A unit without
  that ability shows the tree and courses only. The `un.*` keys exist; add `nav.units` and the page's own keys in both
  catalogues (a missing `uk` key is a type error).

- [ ] **Step 1:** Add the keys, the icon, `navFor`'s option and the page; the "new top-level unit" button is not shown (the
  server allows it only in the console).
- [ ] **Step 2: Verify** `pnpm typecheck && pnpm lint` in `web/`.
- [ ] **Step 3: Commit** `feat(web): the Units page for people who hold a unit role`.

### Task 6: The unit picker

**Files:**
- Create: `web/src/components/units/UnitSelect.tsx`
- Modify: `web/src/components/courses/NewCourse.tsx`, `web/src/components/course/CourseSettings.tsx`, the i18n files

**Interfaces:**
- `UnitSelect({ id, value, onChange, current? })`: a labelled `<select>` of "No unit" plus the `useUnits()` items whose
  `abilities` include `manage_courses`, indented by level; when `current` is not among them it is shown, selected and
  disabled. Renders nothing when the person has no placeable unit and the course has no unit.
- `NewCourse`: shows it beside the term select and passes `unit_id` to both create paths. `CourseSettings` General section:
  shows it when `can.manage_course`, saves through `useSetCourseUnit`, separate from the `PATCH`.

- [ ] **Step 1:** Write the component and both uses.
- [ ] **Step 2: Verify** typecheck and lint; the `/courses/new` screen sweep in `e2e/ux.spec.ts` still passes.
- [ ] **Step 3: Commit** `feat(web): place a course in a unit when creating it and in its settings`.

### Task 7: e2e for the dean and the viewer

**Files:**
- Create: `web/e2e/units.spec.ts`
- Modify: `web/e2e/routes.ts` (`RoleName` and `accounts` gain `dean`, `viewer`; `staticRoutes` adds `/units` for them)

- [ ] **Step 1: Write the tests** (they fail until Tasks 3-6 are in; write them last, so they pass):
  - `a dean sees Units, opens the unit and a course they do not teach, with its gradebook`: sign in as `dean`; the main
    navigation has "Units"; the unit lists the course; opening it shows the gradebook, and `/course/settings` shows the unit
    select.
  - `a viewer sees grades and no edit control`: sign in as `viewer`; the gradebook opens; no "Edit" and no "Add" controls.
  - `a student has no Units link and no unit select` on `/courses/new` is not reachable for students; assert the link only.
  - `a course moves to No unit and back` from settings as the dean.
- [ ] **Step 2: Verify** `cd web && pnpm exec playwright test e2e/units.spec.ts e2e/pages.spec.ts e2e/ux.spec.ts`, and
  `cd web-admin && pnpm exec playwright test`.
- [ ] **Step 3: Commit** `test(web): a dean and a viewer use their units`.

---

## Part B: the activity type contract

### Task 8: The crate `activity` with the document model and the contract data

**Files:**
- Create: `backend/crates/learning/activity/{Cargo.toml,src/lib.rs,src/tests.rs}`; `src/document/` (moved from
  `content/src/document/`, tests included); `src/contract.rs` (`Stage`, `ValidContent`, `AutoGrade`, `Claims`, moved from
  `content/src/types/mod.rs`)
- Modify: `backend/Cargo.toml` (member via the `learning/*` glob; `activity = { path = ... }` in workspace dependencies),
  `content/Cargo.toml`, `content/src/lib.rs` and its imports, `content/src/types/*` (`use activity::...`), the eleven
  outside callers (`flows`, `server/src/app.rs`, `server/tests/content.rs`, `ai_agent/src/tools/mod.rs`),
  `server/tests/architecture.rs`, `CLAUDE.md` section 4 (a line for `activity`), release scripts' crate globs if they list
  crates by name (check `RELEASING.md`)

**Interfaces:**
- Produces: `activity::{document::*, Stage, ValidContent, AutoGrade, Claims}` with the same items and visibility as today
  (`ValidContent::new` becomes `pub`). `content` no longer exports them.
- The architecture test: `activity` is a crate without a schema, its manifest allows only `api-types`, `vumlo-config`,
  `serde_json`, `sqlx`, `thiserror`, `uuid`, `chrono`.

- [ ] **Step 1: Write the failing test** in `server/tests/architecture.rs`: `activity` is listed and may name no other
  crate of ours (extend the existing manifest check by one entry; run it: FAIL, crate missing).
- [ ] **Step 2:** Create the crate, `git mv` the document model and the four shared types, fix the paths inside, change
  `content` to depend on it, update every `content::X` import of a moved name to `activity::X`.
- [ ] **Step 3: Verify** `cargo test -p activity --no-run`, `cargo check --workspace --all-targets --all-features`,
  `cargo test -p activity` (the moved document tests), `cargo test -p content`, `cargo test -p server --test architecture`.
- [ ] **Step 4: Commit** `refactor(activity): the document model and the contract's data move into the crate activity`.

### Task 9: `ActivityType`, `Capabilities`, `CompletionPolicy`

**Files:**
- Create: `activity/src/model.rs` and `activity/src/model/tests.rs`

**Interfaces:**
- Produces (the signatures of spec section 3): `trait ActivityType: Send + Sync`; `struct Capabilities { pub submissions:
  bool, pub auto_graded: bool }` (derives `Clone, Copy, Debug, PartialEq, Eq`); `enum CompletionPolicy
  { SubmittedOrGraded, Graded }` with `fn is_met(self, progress: Progress) -> bool`; `struct Progress { pub submitted:
  bool, pub graded: bool, pub marked: bool }`; `struct ConfigError(String)` (`thiserror`, `Display` is the text,
  `ConfigError::new(impl Into<String>)`).
- The trait's method names and parameters are those of `content::model::AssignmentType` today, `ModelError` replaced by
  `ConfigError`, plus `id`, `capabilities`, `completion`, `config_file_refs(&self, &Map<String, Value>) -> Vec<Uuid>`,
  `submission_file_refs(&self, &Map<String, Value>, &Map<String, Value>) -> Vec<Uuid>`, and `check_file_names(&self, config,
  names: &[String]) -> Result<(), DocumentErrors>`. The last three have a default that returns empty/`Ok`.

- [ ] **Step 1: Write the failing tests** in `model/tests.rs`: `completion_policy_table` asserts, for all eight combinations
  of `Progress`, that `SubmittedOrGraded.is_met == submitted || graded` and `Graded.is_met == graded` (a `marked`-only
  progress is false for both: marking is the item's rule, not a type's, spec section 6).
- [ ] **Step 2:** Run `cargo test -p activity`: FAIL (no item).
- [ ] **Step 3:** Implement the items above. Read `content/src/model.rs:130-176` and keep each method's doc comment.
- [ ] **Step 4:** Run: PASS. **Commit** `feat(activity): the ActivityType contract, its capabilities and completion policy`.

### Task 10: The registry and `Kind`

**Files:**
- Create: `activity/src/registry.rs`, `activity/src/registry/tests.rs`; `activity/Cargo.toml` gets the feature `testing`

**Interfaces:**
- Produces: `struct Types` with `Types::new() -> Self`, `fn with(self, t: &'static dyn ActivityType) -> Result<Self,
  InstallError>` (refuses a duplicate id and an id outside `^[a-z][a-z0-9_]{0,31}$`); `fn install(types: Types) ->
  Result<(), InstallError>` (`AlreadyInstalled`); `fn get(id: &str) -> Result<Kind, RegistryError>`
  (`NotInstalled`, `Unknown(String)`); `fn available() -> Vec<&'static str>` (sorted); `fn ids_where(f: fn(Capabilities) ->
  bool) -> Vec<String>` (owned, to bind as `text[]`); `#[cfg(feature = "testing")] fn ensure_installed(build: impl
  FnOnce() -> Types)`. `Kind`: `Copy, Debug, PartialEq, Eq`, `Deref<Target = dyn ActivityType>`, `fn id(&self)`,
  `sqlx::Type<Postgres>` (as `text`) and `Decode` through `get` (an unknown id is a decode error naming the id), `Encode`
  as the id.

- [ ] **Step 1: Write the failing tests** with two dummy types in `registry/tests.rs`: `unknown_and_not_installed_are_errors`,
  `install_twice_fails`, `duplicate_and_malformed_ids_are_refused`, `ids_where_picks_by_capability`,
  `available_is_sorted`. The registry is a process-wide `OnceLock`: the tests that need an installed registry use
  `ensure_installed`; the ones that need none build a `Types` and call the pure helpers (`Types::get`, `Types::available`),
  which the free functions wrap.
- [ ] **Step 2:** Run `cargo test -p activity --features testing`: FAIL. Implement; run: PASS.
- [ ] **Step 3: Commit** `feat(activity): the registry of activity types`.

### Task 11: The five built-ins implement the contract

**Files:**
- Create: `content/src/types/text.rs`, `types/no_submission.rs` (each with `pub const ID`), a unit struct per type in the
  existing `quiz.rs`, `form.rs`, `file.rs`; `content/src/types/registry.rs` (`builtin_types()`), `types/registry/tests.rs`
- Modify: `content/src/types/mod.rs`, `content/src/model.rs` (`editor_features` moves to `types/text.rs`; `BuiltinType`
  stays until Task 12), `content/src/lib.rs`

**Interfaces:**
- Produces: `pub fn builtin_types() -> Result<activity::Types, activity::InstallError>` in `content` (the roots handle the
  error at startup; no `expect` in a library, CLAUDE.md section 10); per module `pub const ID: &str` and `pub struct Quiz;`
  etc. implementing `ActivityType` by calling the module's existing functions (no logic is rewritten).
  Capabilities: `quiz { submissions: true, auto_graded: true }`, `no_submission { false, false }`, the others `{ true,
  false }`. Policies: `no_submission` -> `Graded`, the rest `SubmittedOrGraded`.

- [ ] **Step 1: Write the failing test** `built_in_table` in `types/registry/tests.rs`: for the five ids, asserts `id`,
  `capabilities` and `completion` exactly as above, and that `builtin_types()` has no other id.
- [ ] **Step 2:** Implement; the existing per-type tests (`types/*/tests.rs`, `model/tests.rs`) keep passing untouched.
- [ ] **Step 3: Verify** `cargo test -p content`. **Commit** `feat(content): the built-in types implement ActivityType`.

### Task 12: `BuiltinType` is replaced by `Kind` across the workspace

**Files:** every file `grep -rl BuiltinType backend` lists (about 25: `content` model/repo/service/routes, `assessment`
service, `gradebook` model and `repo/work.rs`, `flows` items/plugin_read/submissions, `ai_agent` tools, `server`/`vumlo-cli`
tests and seeds); `server/src/app.rs` and `vumlo-cli/src/main.rs` (install); `backend/.sqlx/`

**Interfaces:**
- `Assignment`, `AssignmentSummary`, `OpenedAssignment`, `OpenAssignment`, gradebook `WorkEntry` carry
  `assignment_type: activity::Kind`; queries decode `AS "assignment_type: activity::Kind"`.
- `routes.rs` parses with `activity::get`, mapping `RegistryError::Unknown` to the field `assignment.type` with
  `ModelError::AssignmentType` whose text lists `activity::available()`.
- Type-only features compare with the owning module's `ID` (`quiz::ID`, `form::ID`, `text::ID`, `file::ID`):
  `assessment` `QuizAttemptSubmitted`, `begin_quiz_result`, `responses`; `plugin_read` replaces its `FORM`/`QUIZ` string
  constants; `ai_agent` replaces `== Text`. Those IDs are exported from `content` for the crates above it.
- `takes_submissions()` callers use `capabilities().submissions`; the `== File` file-name check is
  `kind.check_file_names(..)`; `config_file_refs` / `submission_file_refs` are trait calls.
- `server/src/app.rs` (next to `install_subscribers`) and `vumlo-cli/src/main.rs:~434` call
  `activity::install(content::builtin_types()?)`; tests use `ensure_installed(|| content::builtin_types().unwrap())` where
  they use `events::ensure_installed`.

- [ ] **Step 1: Write the failing test** in `server/tests/architecture.rs`: `no_builtin_type_name_is_left` fails while any
  `.rs` under `backend/crates` (tests included) contains `BuiltinType`, and `both_binaries_install_the_types` greps that
  `server/src/app.rs` and `vumlo-cli/src/main.rs` call `activity::install`. Run: FAIL.
- [ ] **Step 2:** Make the replacement crate by crate in the order `content`, `assessment`, `gradebook`, `flows`,
  `ai_agent`, `server`, `vumlo-cli`, checking each with `cargo check -p <crate> --all-targets` (add `--all-features` for
  `flows` and `server`). Delete `BuiltinType`, the `AssignmentType` trait and the `text_enum!` for it.
- [ ] **Step 3:** With Postgres up, `cargo sqlx prepare --workspace -- --all-targets --all-features`; commit `.sqlx`.
- [ ] **Step 4: Verify** `cargo check --workspace --all-targets --all-features`; `cargo test -p content`; `cargo test -p
  server --test content`, `--test facts`, `--test gradebook`, `--test home`, `--test architecture`, each alone.
- [ ] **Step 5: Commit** `refactor(learning): assignment types come from the registry, BuiltinType is gone (ADR-0097)`.

### Task 13: Capabilities replace the type literals in SQL

**Files:**
- Modify: `gradebook/src/repo/work.rs` (`student_progress`, `student_next`, `todo_counts`), `gradebook/src/repo.rs`
  (`unpublished_grades`), `vumlo-cli/src/seed.rs:~262`
- Test: `server/tests/gradebook.rs` and `server/tests/home.rs` (existing assertions) plus one new test

**Interfaces:**
- The queries bind `activity::ids_where(|c| !c.submissions)` and `ids_where(|c| c.auto_graded)` as `&[String]` and use
  `a.type <> ALL($n)` / `a.type = ANY($n)`. The seed uses `quiz::ID`.

- [ ] **Step 1: Write the failing test** `submissionless_types_come_from_the_registry` in `server/tests/gradebook.rs`: install
  a registry that adds a dummy type `in_person` (`submissions: false`, `Graded`) next to the built-ins
  (`Types::with`), create an assignment of that type, and assert it is listed as in-class work and is excluded from the
  student's next work. Run: FAIL (the SQL still names `no_submission`).
- [ ] **Step 2:** Change the five queries; keep every column alias. Re-run `cargo sqlx prepare` as in Task 12.
- [ ] **Step 3: Verify** the test and `--test gradebook`, `--test home` PASS. **Commit**
  `refactor(gradebook): work lists ask the types' capabilities, not their names`.

### Task 14: Completion is decided by the type's policy

**Files:**
- Modify: `content/src/repo.rs` (`facts`, ~line 1515), `.sqlx`
- Test: `server/tests/facts.rs`, `content/src/repo` or `rules` tests as the crate keeps them

**Interfaces:**
- `facts(conn, course_id, student_id) -> Facts` selects per fact row `submitted_at`, `published_score`, `marked_at` and the
  item's assignment type (`LEFT JOIN content.assignments`), builds `Progress` and sets `done_items` as: assignment ->
  `kind.completion().is_met(progress)`, other item -> `marked`.

- [ ] **Step 1: Write the failing tests** in `server/tests/facts.rs` with `db_test`:
  `a no_submission assignment is done only when its score is published`, `a quiz is done at submit, before grading`,
  `a text item is done when marked`. The first fails first only if a row with `submitted_at` could exist for it; seed one
  directly (the table's CHECK allows it) and expect "not done", which fails against "a row means done".
- [ ] **Step 2:** Implement; `rules.rs` is untouched. Re-run `cargo sqlx prepare`.
- [ ] **Step 3: Verify** `--test facts`, `--test access_rules`, `--test content`, then `vumlo-cli facts rebuild`'s test in
  `--test cli` if it names completion. **Commit** `feat(content): an item is done by its type's completion policy`.

---

## Part C: finish

### Task 15: Docs and the full check

**Files:**
- Modify: `docs/superpowers/roadmap-0.9.0.md`, `docs/decisions/{in-progress,done}.md`, `ARCHITECTURE.md`, `docs/PLAN.md`
  if they name `BuiltinType`, `RELEASING.md` if a crate list names crates

- [ ] **Step 1:** Roadmap: items 1-3 built, the admin console and the installer (ADR-0095, ADR-0096) listed, a state for the
  PR. `in-progress.md`: the ADR-0094 line loses "Missing: the web pages" and moves to `done.md`; ADR-0097 line says what is
  built; ADR-0008's mention of the future plugin boundary points to ADR-0097. `grep -rn BuiltinType docs ARCHITECTURE.md`
  has no live hit.
- [ ] **Step 2: Run the definition of done** (CLAUDE.md section 14), each alone, `CARGO_BUILD_JOBS=2`: `cargo fmt --all
  --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo sqlx prepare --workspace --check
  -- --all-targets --all-features`, `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; web:
  `pnpm typecheck && pnpm lint && pnpm build` in `web/` and `web-admin/`. The full `cargo test --workspace` is left to CI
  (`git diff --stat` and the targeted tests above are the local proof).
- [ ] **Step 3: Commit** `docs: roadmap and decisions after units in the web app and activity types`. Stop: the owner
  reviews and pushes.

## Self-review notes

- Spec coverage: org-units section 8 (page, picker, i18n, `can`, e2e) -> Tasks 1-7; activity spec sections 2-7 -> Tasks 8-14;
  section 8 tests -> 9-14; docs -> 0 and 15. `web/` needs no regeneration of `schema.d.ts` (PR A did it).
- `activity` has no schema and no migration: nothing for `architecture.rs`'s view list.
- Risk: Task 12 is the one large mechanical step; its failing architecture test is the guard that nothing is missed.
