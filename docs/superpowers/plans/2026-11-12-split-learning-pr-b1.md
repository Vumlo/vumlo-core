# Split `learning`, PR B1: nothing in a lower module reads or calls a higher one. Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Inside `learning`, order the four modules `teaching < content < assessment < gradebook < flows` and remove every SQL read and every Rust call that goes up that order, by moving each read model to the module that sits above what it reads, or to a flow in `learning` that composes the owners' functions.

**Architecture:** `architecture.rs` gets one layer per module and a ratchet: a list of the violations that exist today, which the test tolerates and requires to shrink (an entry that no longer occurs fails the test). Each task deletes its entries first (red), then moves the code (green). The API does not change: a moved handler keeps its path, DTOs and `operation_id`.

**Tech Stack:** Rust, sqlx `query!`, axum, utoipa. No migration, no new crate.

**Spec:** `docs/superpowers/specs/2026-11-12-split-learning-design.md` (sections 3 to 5; PR B is split in two: B1 here removes what goes up, B2 declares the views for what goes down). ADR-0093. PR A is merged first (it removed the reads of `content` into `assessment` and `gradebook` that the access rules had).

## Global Constraints

- Run from `backend/`; targeted runs, `CARGO_BUILD_JOBS=2`, one command at a time. The full workspace and `--features dummy` runs are left to CI. Dev database and `cargo sqlx prepare --workspace -- --all-targets --all-features` as in PR A's plan.
- The API is unchanged: `GET /api/v1/openapi.json` (`jq -S .`) stays identical to `web/openapi.json`, and no existing test changes an assertion.
- SQL only in `repo.rs`; services take `&mut PgConnection`; authorize before revealing anything (CLAUDE.md sections 4 and 5). A moved function keeps its locking order (ADR-0030, ADR-0035: lock the assignment rows, then ask the other modules).
- Move code with `git mv` where a file moves; no copy left behind, no re-export shim for the old path (CLAUDE.md section 13).
- Branch `feat/split-boundaries`, stacked on `feat/split-learning` (PR #61). Conventional Commits.

## Review Focus

- A moved handler that loses its authorization or its lock: course delete, module and item delete, item update (the guards must still run under the assignment row lock).
- A read model that starts showing a draft grade because it moved above `gradebook`: `/me/course-cards` staff counts, `/me/work`.
- The ratchet itself: an entry that is fixed but left in the list, or a new violation hidden by a too wide entry.
- Behaviour of `GET /courses/{id}/info` and `/people` for off-roster students (the former count and list must stay equal).
- `GET /items/{id}` for a form: the decorated config must be the same for staff, a student, and a locked item.

## File Structure

- Modify `server/tests/architecture.rs` (layers, ratchet, import scan).
- Create flow modules `learning/src/courses/{mod.rs,model.rs,routes.rs,service.rs}` and `learning/src/items/{mod.rs,routes.rs,service.rs}`; modify `learning/src/lib.rs`, `server/src/app.rs` (routers).
- Move read-model SQL: `teaching/repo/cards.rs` and parts of `teaching/repo/mod.rs` into `gradebook/repo/` and `content/repo.rs`; `content/repo.rs::{student_work,staff_counts}` into `gradebook/repo/`.
- Modify `teaching/service/{mod,info}.rs`, `content/service.rs`, `assessment/service.rs`, their `mod.rs`, `model.rs`, error enums, and `ARCHITECTURE.md`, `CLAUDE.md` section 4, `docs/decisions/in-progress.md`.

---

### Task 1: Layers and the ratchet in `architecture.rs`

**Files:** Modify `server/tests/architecture.rs`.

**Interfaces:** Produces `UNITS` with layers `teaching 7, content 8, assessment 9, gradebook 10, learning 11, plugins 12, server 13` (the `learning` unit keeps no schema); `const RATCHET_UP: &[(&str, &str)]` and `const RATCHET_DOWN: &[(&str, &str)]` of `(unit name, schema.table)`; `const RATCHET_CALLS: &[(&str, &str)]` of `(unit name, higher module)`; `DECLARED_VIEWS` gains `teaching.effective_members` (it exists since ADR-0033).

- [ ] **Step 1: Write the failing tests.** In `architecture.rs`: change the layers; make `a_unit_reads_its_own_layer_and_the_declared_views_below_it` tolerate a violation only if `(unit, table)` is in `RATCHET_UP` (higher layer) or `RATCHET_DOWN` (lower layer, not a declared view), and fail when a ratchet entry has no occurrence (`the_ratchet_lists_only_what_still_happens`). Add `a_module_calls_only_modules_below_it`: scan the non-test `.rs` files of `learning/src/{teaching,content,assessment,gradebook}` for `crate::<module>::` and `super::<module>::` paths that name a higher module, tolerated only through `RATCHET_CALLS`. Start the lists from this output of a throwaway script (not committed): `RATCHET_UP` = `content`: `assessment.submissions`, `gradebook.grades`; `teaching`: `content.assignments`, `content.items`, `content.modules`, `assessment.submissions`, `gradebook.grades`. `RATCHET_CALLS` = `teaching`: `content`, `assessment`, `gradebook`; `content`: `assessment`, `gradebook`, `teaching`-free; `assessment`: `gradebook`. `RATCHET_DOWN` = every other cross-schema read the test then reports (run it, copy the report, group by `(unit, table)`; about 35 entries; B2 empties it).
- [ ] **Step 2:** `cargo test -p server --test architecture` passes only with the lists complete; fix any entry the test reports until green. Temporarily remove one `RATCHET_UP` entry to confirm the test goes red naming it, then put it back.
- [ ] **Step 3: Commit** `test(architecture): layers inside learning and a ratchet of the reads and calls that go up (ADR-0093)`.

### Task 2: `/me/work` moves to `gradebook`

**Files:** Modify `content/{repo,service,mod,model,routes}.rs`; create/modify `gradebook/{repo,service,mod,routes}` accordingly.

**Interfaces:** `gradebook::my_work(conn, actor, ...)` with the signature `content::my_work` has today; the types `MyWork`, `WorkFacts`, `WorkStatus` stay in `content::model` (a higher module may use them). `content::repo::{student_work, staff_counts}` become `gradebook` repo functions, same SQL. The handler of `GET /me/work` moves to `gradebook/routes`, same path and annotation.

- [ ] **Step 1:** delete the two `content` entries of `RATCHET_UP`; `cargo test -p server --test architecture` fails naming `student_work` and `staff_counts`.
- [ ] **Step 2:** move `student_work`, `staff_counts`, the service function(s) that call them (find with `grep -rn "student_work\|staff_counts" crates/learning/src`) and the route; wire the router in `server/src/app.rs` like the other `gradebook` routes; update the `use` lines; remove the dead imports of `content`.
- [ ] **Step 3:** `cargo test -p server --test architecture --test home --test content --test gradebook` green; `cargo clippy -p learning --all-targets -- -D warnings`.
- [ ] **Step 4: Commit** `refactor(gradebook): the read model of /me/work lives above what it reads (ADR-0093)`.

### Task 3: `/me/course-cards` becomes the flow `learning::courses::cards`

**Files:** Create `learning/src/courses/{mod.rs,model.rs,routes.rs,service.rs}`; modify `teaching/repo/cards.rs`, `teaching/service/*`, `teaching/routes/*`, `content/{repo,service,mod}.rs`, `gradebook/{repo,service,mod,model}.rs`.

**Interfaces:**
- `gradebook::student_progress(conn, student_id, course_ids, now) -> Vec<Progress>`, `gradebook::student_next(conn, student_id, course_ids, now) -> Vec<NextDeadline>`, `gradebook::todo_counts(conn, course_ids) -> Vec<TodoCounts>` (the SQL of `teaching/repo/cards.rs::{student_progress, student_next, staff_todo_counts}` unchanged; the draft comparison stays inside `gradebook`, which owns drafts). The types `Progress`, `NextDeadline`, `TodoCounts` move with them.
- `content::draft_items(conn, course_ids) -> Vec<DraftItem>` (from `unpublished_items`) and `content::next_deadlines(conn, course_ids, now) -> Vec<NextDeadline>` (from `staff_next_deadline`).
- `teaching` keeps the course rows with their roster counts (the first function of `cards.rs`).
- `learning::courses::cards(conn, actor, now) -> Result<Vec<CourseCard>, LearningError>` composes the four; the handler of `GET /me/course-cards` moves to `learning::courses::routes`.

- [ ] **Step 1:** delete the `teaching` entries of `RATCHET_UP` for `content.modules`, `content.assignments`, `gradebook.grades` that belong to `cards.rs` (the test names what is left); red.
- [ ] **Step 2:** move the SQL and types as above; build the flow and the route; the card DTO is assembled in the flow exactly as `teaching::cards` did.
- [ ] **Step 3:** `cargo test -p server --test architecture --test home --test teaching --test courses` green; sqlx prepare; clippy.
- [ ] **Step 4: Commit** `refactor(learning): course cards are a flow over the owners' read models (ADR-0093)`.

### Task 4: course lifecycle and the header become flows in `learning::courses`

**Files:** `learning/src/courses/*`; `teaching/service/{mod,info}.rs`, `teaching/routes/info.rs`, `teaching/repo/mod.rs`, `teaching/service/mod.rs` (error enum), `content/{service,mod}.rs`, `gradebook/{service,mod}.rs`, `assessment/{service,mod}.rs`.

**Interfaces:**
- Flows (same paths and DTOs as today): `courses::publish(conn, actor, course_id)` = `teaching::publish_course` then `content::announce_visible(conn, now, Some(course_id))`; `courses::delete(conn, actor, course_id)` = authorize through `teaching`, `content::lock_course_assignments`, `gradebook::has_grades_in_course`, `assessment::has_submitted_in_course` (409 as today), `gradebook::purge_course`, `assessment::purge_course`, then `teaching::delete_course`; `courses::create_template` and `courses::create_from_template` = the `teaching` row plus `content::copy_course_content` plus `gradebook::copy_layout`; `courses::info(conn, actor, course_id)` and `courses::people(...)`; `courses::templates(conn, actor, page)`.
- Owner functions: `gradebook::students_with_values(conn, course_id) -> Vec<Uuid>` (anyone with a draft or published score or feedback), `assessment::students_with_attempts(conn, course_id) -> Vec<Uuid>` (a submitted attempt), `teaching::former_people(conn, course_id, candidates: &[Uuid]) -> Vec<Person>` (those candidates who are not on the roster, ordered by surname, name, id), `content::template_counts(conn, course_ids) -> Vec<TemplateCounts { course_id, sections, lectures, assignments }>`, `content::course_summary` keeps its signature. `teaching::InfoRow` loses `former_count` (the flow takes the length of `former_people`); `teaching::TemplateSummary` loses its three counts (the flow adds them).
- `TeachingError` loses its `Content`, `Gradebook` and `Assessment` variants and the `from_*` mappers in `teaching/service/info.rs`; the flows' error is `LearningError`, which gains variants for them.

- [ ] **Step 1:** delete the remaining `RATCHET_UP` entries of `teaching` and the `RATCHET_CALLS` entries of `teaching`; red.
- [ ] **Step 2:** write the owner functions, move the SQL (`info_row` loses its two subqueries; `list_former` and `list_templates` are replaced by the owner functions above), write the flows and move the routes. Keep the order of locks of `delete_course` exactly.
- [ ] **Step 3:** `cargo test -p server --test architecture --test courses --test teaching --test groups_roster --test admin_reports --test plugin_events` green; compare `courses::info` of a course with an off-roster student before and after (the existing tests of ADR-0037 cover it; add `former_count_equals_the_length_of_the_former_list` to `server/tests/courses.rs` if none does).
- [ ] **Step 4:** sqlx prepare; clippy; **Commit** `refactor(learning): course publish, delete, templates and header are flows (ADR-0093)`.

### Task 5: deleting, updating and reading items become flows in `learning::items`

**Files:** `learning/src/items/*`; `content/{service,mod,model,routes}.rs`, `assessment/{service,mod,routes}.rs`.

**Interfaces:**
- Two-phase delete in `content`: `begin_delete_module(conn, actor, id) -> Result<ModuleDeletion, ContentError>` (authorize through `manage_module`, then `repo::lock_assignments_below`) and `finish_delete_module(conn, ModuleDeletion)` (the delete, `ModuleNotFound` when it lost the race); the same for items (`begin_delete_item`, `finish_delete_item`, locking the assignment row `FOR UPDATE`). The flow `items::delete_module` / `items::delete_item` runs begin, `gradebook::has_grades_in_*` (`HasGrades`), `assessment::has_submitted_*` (`HasSubmissions`), the two `purge_*`, finish, and writes the audit row where `content` wrote it.
- Update: `content::begin_update_item(conn, actor, id, &patch) -> Result<ItemUpdate, ContentError>` authorizes and, for an assignment patch that touches points or config, takes `repo::lock_assignment` first; the flow reads `AssignmentFacts { has_submissions: bool, highest_score: Option<Decimal> }` from `assessment::has_submitted_for_assignment` and `gradebook::max_score`, then `content::finish_update_item(conn, actor, rules, update, facts)` which raises `StructureFrozen` and `BelowGivenScores` from the facts. The handler of `PATCH /items/{id}` moves to the flow.
- Read: `content::get_item` returns `ItemRead { item, form: Option<FormView { staff: bool }> }`, `form` set only for an unlocked form; `content::decorate_form(item: &mut Item, view: FormView, answered, holders: &Holders, viewer: Uuid)` is the `form::decorate` call; the flow `items::get` adds `assessment::form_facts`. The handler of `GET /items/{id}` moves.
- Result: the handler of `GET /assignments/{id}/result` (`assessment::quiz_result`, which asked `gradebook::published_grade`) becomes `items::result`: it reads the published grade through `gradebook` and passes it to `assessment::quiz_result(conn, actor, id, published: Option<PublishedGrade>)`.

- [ ] **Step 1:** delete the remaining `RATCHET_CALLS` entries (`content`, `assessment`); red.
- [ ] **Step 2:** implement the interfaces; keep every lock where it is relative to the guards.
- [ ] **Step 3:** `cargo test -p server --test architecture --test content --test submissions --test gradebook --test quiz_and_form --test revisions --test access_rules --test plugin_events` green. Add `a_form_item_reads_the_same_config_for_staff_a_student_and_a_locked_item` to `server/tests/quiz_and_form.rs` if the existing form tests do not already compare the three.
- [ ] **Step 4:** sqlx prepare; clippy; **Commit** `refactor(learning): item delete, update and read are flows (ADR-0093)`.

### Task 6: close B1

**Files:** `server/tests/architecture.rs`, `ARCHITECTURE.md`, `CLAUDE.md`, `docs/decisions/in-progress.md`, `docs/superpowers/specs/2026-11-12-split-learning-design.md`.

- [ ] **Step 1:** `RATCHET_UP` and `RATCHET_CALLS` are empty: delete both constants and make the two tests strict; add a test that `learning` modules never mention a higher module's schema (the same scan as the first test, no allowlist).
- [ ] **Step 2:** docs: `ARCHITECTURE.md` (crates table rule 2: the four modules are ordered; the new flows `courses` and `items`; the route owners), `CLAUDE.md` section 4 (the order inside `learning`), the ADR-0093 line ("Built: PR A, B1"), the spec (B1/B2 split).
- [ ] **Step 3: Checks:** `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test -p server --test architecture --test authorization`; `cargo sqlx prepare --workspace --check -- --all-targets --all-features`; `SQLX_OFFLINE=true cargo check --workspace --all-targets`; OpenAPI identical (run the server with the scratch config, `curl`, `jq -S`, `diff` against `web/openapi.json`).
- [ ] **Step 4:** self-review of the diff for leftovers (no `// moved` comments, no unused re-exports, no old error variants), push, open the PR (base `feat/split-learning` until #61 merges), stop for review.

---

## Outline of B2 and C (own plans)

B2 (migration `0054`): declare the views for the reads that go down and empty `RATCHET_DOWN`. Readers and tables, from the ratchet: `assessment` and `gradebook` read `content` items, modules, assignments and `teaching` courses and `effective_members`; `gradebook` reads `assessment` submissions; the flows (`plugin_read`, `files`, `audit`, `facts`) read all four, including `gradebook.grades` through a **published-only** view. Needs a decision per view on its columns, taken from the SQL that uses it.

C: `git mv` into `backend/crates/learning/{teaching,content,assessment,gradebook,flows}`, five crates with their own errors, state and routers; CI paths, `Dockerfile`, `RELEASING.md`, docs.
