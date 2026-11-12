# Split `learning`, PR A: event dispatch and the facts projection. Implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `events::emit` also runs synchronous subscribers in the caller's transaction, and `content` keeps its own `content.facts` projection (updated by those subscribers, rebuildable) so the access rules stop reading `assessment` and `gradebook`.

**Architecture:** `events` gets an immutable registry of synchronous subscribers, installed once by the composition root; `emit` writes the outbox row and then runs the subscribers of that event kind on the same connection. `learning::facts` (a flow, still inside the one `learning` crate) reads the sources of truth and tells `content` to replace its facts; the subscribers and the rebuild call the same function. The quiz auto-grade (`assessment` to `gradebook`) becomes a subscriber of `quiz.attempt_submitted`.

**Tech Stack:** Rust (sqlx `query!` macros, axum, tokio), PostgreSQL, `std::sync::OnceLock`. No new crates (ADR-0014).

**Spec:** `docs/superpowers/specs/2026-11-12-split-learning-design.md` (sections 6, 7, and PR A of section 8). ADR-0093.

## Global Constraints

- Run from `backend/`. Local runs are targeted and one at a time: `CARGO_BUILD_JOBS=2 cargo test -p <crate> ...`. The full workspace and `--features dummy` runs are left to CI.
- Dev database: `docker compose up -d postgres`; compile-time checked queries need `DATABASE_URL=postgres://vumlo:vumlo@localhost:5432/vumlo` and migrations applied with `sqlx migrate run --source migrations` (not `vumlo-server migrate`: the local `vumlo.toml` has `database.url`, which the config refuses).
- Migrations are append-only, schema-qualified, enums are `text` + `CHECK`, scores `numeric`, ids UUID v7 (CLAUDE.md §8). After any query change: `cargo sqlx prepare --workspace -- --all-targets`, commit `backend/.sqlx/`.
- SQL only in `repo.rs`; no `unwrap`/`expect`/`panic!` outside tests; unit tests live in `foo/tests.rs` declared by `#[cfg(test)] mod tests;`, DB tests use `db_test(async |pool| { ... })` (CLAUDE.md §10, §12).
- Event payloads hold ids only (CLAUDE.md §7). A student never receives a draft grade field (CLAUDE.md §6): the facts hold the published score only.
- API and behaviour do not change. `GET /api/v1/openapi.json` stays byte-identical, and no existing test changes an assertion.
- Change files with Edit/Write, not python/sed/perl in Bash. Never commit `graphify-out/`. Never `pkill -f` a text that is in your own command line.
- Branch `feat/split-learning` (already has the spec commit). Conventional Commits.

## Review Focus

- **Two transactions for one pair at once** (a submit and a publish for the same student and assignment): neither may overwrite the other's column with a stale value. Pinned by Task 4's lock test.
- **A grade retracted to an empty value** (publish with an empty draft) clears `published_score`; today it emits nothing. Pinned by Task 4.
- **A draft grade or an unpublished score** never reaches `content.facts`. Pinned by Task 4.
- **A submit made by the scheduler (auto-submit)** produces the same facts as a student's submit, because it goes through the same code. Pinned by Task 4.
- **A process that emits with no registry installed** fails loudly (a worker, a CLI command, a unit-test binary). Pinned by Tasks 2 and 3, and the demo seed by Task 7.

## File Structure

- Create `events/src/dispatch.rs`, `events/src/dispatch/tests.rs`: `Subscribers`, `SyncHandler`, `SubscriberError`, the global registry, `install`, `emit_with`.
- Modify `events/src/service.rs` (`emit`), `events/src/lib.rs` (exports, `testing` module), `events/src/model.rs` (`Event::GradePublicationChanged`), `events/Cargo.toml` (feature `testing`).
- Create `learning/src/facts/{mod.rs,service.rs,repo.rs,tests.rs}`: sources, `sync`, `rebuild_facts`.
- Create `learning/src/quiz_grade.rs` (+ `quiz_grade/tests.rs`): the auto-grade subscriber.
- Modify `learning/src/lib.rs` (`subscribers()`, exports, a `#[cfg(test)] mod testing`), `content/{repo,service,standing,mod,model}.rs`, `assessment/{repo,service,mod}.rs`, `gradebook/service.rs`.
- Create `migrations/0051_content_facts.sql`, `0052_drop_completions.sql`, `0053_auto_total.sql`.
- Modify `server/src/app.rs` (install), `vumlo-cli` (`facts rebuild`, seed), docs.

---

### Task 1: The synchronous dispatcher in `events`

**Files:** Create `events/src/dispatch.rs`, `events/src/dispatch/tests.rs`. Modify `events/src/lib.rs`, `events/src/service.rs`, `events/Cargo.toml`.

**Interfaces:**
- Produces (all `pub`, re-exported from `events`):
  - `type SyncFuture<'a> = Pin<Box<dyn Future<Output = Result<(), SubscriberError>> + Send + 'a>>`
  - `struct SubscriberError(pub String)` (like `HandlerError`)
  - `struct Subscribers` (`Clone + Default`), `Subscribers::new() -> Self`,
    `Subscribers::on<F>(self, kind: &'static str, handler: F) -> Self where F: for<'a> Fn(&'a mut PgConnection, &'a Event) -> SyncFuture<'a> + Send + Sync + 'static`, `Subscribers::merge(self, other: Self) -> Self`
  - `fn install(subscribers: Subscribers) -> Result<(), EventsError>`
  - `async fn emit_with(conn: &mut PgConnection, event: Event, subscribers: &Subscribers) -> Result<(), EventsError>`
  - `EventsError` gains `AlreadyInstalled`, `NoSubscribers`, `Subscriber(SubscriberError)`; all three map to a logged internal error in `From<EventsError> for ApiError`, never exposing the text.
  - Feature `testing`: `events::testing::ensure_installed(build: impl FnOnce() -> Subscribers)`, which installs only if nothing is installed yet and never fails.
- In this task `emit` is unchanged (it still only writes the outbox row), so the rest of the workspace stays green.

- [ ] **Step 1: Write the failing tests** in `dispatch/tests.rs` (use `db_test`, `Event::GradePublished`, `Event::EnrollmentChanged`):
  - `subscribers_run_in_registration_order_after_the_outbox_row_is_written`: two subscribers on `grade.published` push `1` and `2` to a shared `Mutex<Vec<_>>`; the first also asserts, through the same connection, that the outbox row of the event already exists. After `emit_with`, the vector is `[1, 2]`.
  - `a_subscriber_of_another_kind_is_not_run`.
  - `a_subscriber_error_is_returned_and_the_caller_can_roll_back`: open a transaction, `emit_with` with a failing subscriber returns `Err(EventsError::Subscriber(_))`; after rollback `events.outbox` has no row.
  - `the_second_subscriber_does_not_run_after_the_first_failed`.
  - In its own integration-test binary `events/tests/install.rs` (the registry is process-global, so it needs a process of its own): `install_twice_is_an_error` (the second call is `Err(EventsError::AlreadyInstalled)`) and `emit_runs_the_installed_subscribers` (after one `install`, `emit` runs the subscriber; this one is added in Task 2 when `emit` reads the registry).
- [ ] **Step 2:** `CARGO_BUILD_JOBS=2 cargo test -p events --lib dispatch` fails to compile (types missing).
- [ ] **Step 3: Implement** `dispatch.rs` as specified. The registry is a `static REGISTRY: OnceLock<Subscribers>`; `install` is `set` mapped to `AlreadyInstalled`; the testing helper is `get_or_init`. `emit_with` calls `repo::insert_outbox` and then each handler of `event.kind()` in order, stopping at the first error. Add the `testing` feature to `events/Cargo.toml` (like `access`).
- [ ] **Step 4:** the tests of step 1 pass; `cargo test -p events --lib` passes; `cargo clippy -p events --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `feat(events): synchronous subscribers that run in the caller's transaction (ADR-0093)`.

### Task 2: Install the registry in every process, then make `emit` require it

**Files:** Modify `events/src/service.rs`, `server/src/app.rs`, `learning/src/lib.rs`, `learning/Cargo.toml`, `auth/Cargo.toml`, `org/Cargo.toml`, `plugins/Cargo.toml`, and every `tests.rs` of those crates that emits. Test: `events/src/service/tests.rs`, `server/tests/common/mod.rs` (only if a binary needs it).

**Interfaces:**
- Consumes: Task 1.
- Produces: `learning::subscribers() -> events::Subscribers` (empty for now; Tasks 4 and 6 fill it); `server::app::install_subscribers()` (idempotent for the process through a `std::sync::Once`; builds `learning::subscribers()` merged with `crate::modules` subscribers if any, none today) called by `AppState::new`; `emit(conn, event)` reads the installed registry and returns `EventsError::NoSubscribers` when none is installed; in each of `auth`, `org`, `plugins`, `learning` a `#[cfg(test)] mod testing` with `pub async fn db_test(body: impl AsyncFnOnce(PgPool))` that calls `events::testing::ensure_installed(...)` (empty for `auth`, `org`, `plugins`; `crate::subscribers` for `learning`) and delegates to `test_support::db_test`.

- [ ] **Step 1: Write the failing test** in `events/src/service/tests.rs`: `emit_without_an_installed_registry_is_an_error` using `emit` in a test binary that never installs (the existing tests there switch to `emit_with(.., &Subscribers::new())`), asserting `Err(EventsError::NoSubscribers)`.
- [ ] **Step 2:** run `cargo test -p events --lib`; the new test fails because `emit` still succeeds.
- [ ] **Step 3: Wire the roots.** `AppState::new` calls `install_subscribers()`. Add the `testing` wrapper `db_test` to `auth`, `org`, `plugins` and `learning`; in each unit-test file of those crates that emits (find them by running the crate's lib tests, which now fail with `NoSubscribers` after step 4), change `use test_support::db_test` to `use crate::testing::db_test`. `events/testing` is enabled only under `[dev-dependencies]` of those crates (the architecture test enforces it).
- [ ] **Step 4: Flip `emit`** in `service.rs` to `emit_with(conn, event, installed()?)`, where an absent registry is `NoSubscribers`.
- [ ] **Step 5:** run, one crate at a time: `cargo test -p events --lib`, `-p auth --lib`, `-p org --lib`, `-p plugins --lib`, `-p learning --lib`; fix each `NoSubscribers` failure by switching to the wrapper. Then `cargo test -p server --test api --test plugin_events --test registration` (they start the server; `AppState::new` installs). If `vumlo-cli` tests (`cargo test -p vumlo-cli --test commands`) reach an emitting service, install in `vumlo-cli`'s `main` through `server::app::install_subscribers()`.
- [ ] **Step 6:** `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test -p server --test architecture`.
- [ ] **Step 7: Commit** `feat(events): emit runs the installed subscribers and fails without a registry (ADR-0093)`.

### Task 3: The `content.facts` table and the content side of the projection

**Files:** Create `migrations/0051_content_facts.sql`. Modify `content/{repo,service,mod,model}.rs`. Test: `server/tests/access_rules.rs` stays unchanged; new unit tests in `content/service/tests.rs` (create it if absent, declared from `service.rs`).

**Interfaces:**
- Produces (in `content`, re-exported):
  - `enum FactScope { Pair { item_id: Uuid, student_id: Uuid }, Course(Uuid), All }`
  - `struct SourceFact { item_id: Uuid, student_id: Uuid, submitted_at: Option<DateTime<Utc>>, published_score: Option<Decimal> }`
  - `async fn lock_facts(conn: &mut PgConnection, scope: &FactScope) -> Result<(), ContentError>`: for `Pair` an advisory transaction lock on that pair; for `Course` and `All`, `LOCK TABLE content.facts IN SHARE ROW EXCLUSIVE MODE`.
  - `async fn replace_source_facts(conn: &mut PgConnection, scope: &FactScope, rows: &[SourceFact]) -> Result<(), ContentError>`: inside the scope, sets `submitted_at` and `published_score` to NULL, upserts `rows`, then deletes the rows of the scope whose three columns are all NULL. Leaves `marked_at` alone. It authorizes nothing.
- Table (migration `0051`): `content.facts(item_id uuid NOT NULL REFERENCES content.items(id) ON DELETE CASCADE, student_id uuid NOT NULL REFERENCES auth.users(id), submitted_at timestamptz, published_score numeric, marked_at timestamptz, PRIMARY KEY (item_id, student_id), CHECK (submitted_at IS NOT NULL OR published_score IS NOT NULL OR marked_at IS NOT NULL))`. The migration backfills it from `assessment.submissions` (first submitted attempt), `gradebook.grades` (`published_score IS NOT NULL`) and `content.completions` (`marked_at = at`). It keeps `content.completions` for now.

- [ ] **Step 1: Write the failing tests** (`db_test`, seed a course, an assignment and a student with the helpers of the existing content unit tests or by SQL):
  - `replace_in_a_pair_scope_sets_the_two_columns_and_keeps_the_mark`: a row with `marked_at` set; `replace_source_facts(Pair, [submitted_at: t, published_score: 7])` leaves `marked_at`; a second call with an empty `rows` clears both source columns and keeps the row because of the mark.
  - `replace_deletes_a_row_left_with_nothing`.
  - `replace_in_a_course_scope_does_not_touch_other_courses`.
  - `deleting_the_item_deletes_its_facts` (cascade).
- [ ] **Step 2:** fails (no table, no functions).
- [ ] **Step 3: Implement** the migration and the `repo.rs`/`service.rs` functions (queries with `query!`; the scope branches are three fixed statements, no dynamic SQL).
- [ ] **Step 4:** apply the migration to the dev DB, `cargo sqlx prepare --workspace -- --all-targets`, run the tests of step 1 (`cargo test -p learning --lib content`), `cargo test -p server --test migrations`.
- [ ] **Step 5: Commit** `feat(content): content.facts, the projection the access rules will read (ADR-0093)`.

### Task 4: `learning::facts`, the grade event and the subscribers

**Files:** Create `learning/src/facts/{mod.rs,service.rs,repo.rs,tests.rs}`. Modify `learning/src/lib.rs` (`subscribers()`, `pub use facts::rebuild_facts`), `events/src/model.rs`, `gradebook/service.rs` (`publish_grades`), plugin event catalogue (see step 4).

**Interfaces:**
- Consumes: Task 3 (`FactScope`, `SourceFact`, `lock_facts`, `replace_source_facts`), Task 2 (`learning::subscribers`).
- Produces:
  - `Event::GradePublicationChanged { assignment_id, student_id, course_id }`, kind `grade.publication_changed`, version 1, ids-only payload.
  - `learning::facts::sync(conn: &mut PgConnection, scope: FactScope) -> Result<(), LearningError>`: `content::lock_facts`, then reads the sources for the scope (`repo::sources`: per `(assignment, student)` the earliest `submitted_at` of `assessment.submissions` with `status = 'submitted'`, and `gradebook.grades.published_score` where not NULL), then `content::replace_source_facts`.
  - `learning::rebuild_facts(conn: &mut PgConnection, course_id: Option<Uuid>) -> Result<(), LearningError>` = `sync` with `Course(id)` or `All`.
  - `learning::subscribers()` registers `submission.submitted` and `grade.publication_changed`, each calling `sync(Pair { item_id: assignment_id, student_id })`.
  - `gradebook::publish_grades` emits `GradePublicationChanged` for every published row (including a retraction to an empty value); `GradePublished` is unchanged.

- [ ] **Step 1: Write the failing tests** (`facts/tests.rs`, through the services like `learning/src/gradebook/service/tests.rs` does; use `crate::testing::db_test`):
  - `a_submit_sets_submitted_at_and_a_second_attempt_keeps_the_first`.
  - `publishing_a_grade_sets_the_published_score_and_republishing_changes_it`.
  - `retracting_a_grade_to_an_empty_value_clears_the_score`.
  - `a_draft_grade_never_reaches_the_facts` (save a draft score, no publish: no row; also asserts the column names of `content.facts` hold no `draft`).
  - `a_scheduler_auto_submit_sets_the_same_facts_as_a_student_submit` (call `learning::schedule::tick`, as `server/tests/schedule.rs` does, or the `auto_submit` function it calls).
  - `rebuild_equals_the_projection_after_a_scenario`: run submit, publish, republish, retract, mark done, unmark; snapshot `content.facts`; delete every row except the marks; `rebuild_facts(None)`; the snapshot is equal.
  - `rebuild_does_not_touch_marks`.
  - `two_transactions_for_one_pair_do_not_overwrite_each_other`: transaction A runs `sync(Pair)` and stays open; transaction B starts a `sync(Pair)` for the same pair and must block (`test_support::wait_until_blocked`); A commits, B finishes; the row has both columns from the sources.
- [ ] **Step 2:** `cargo test -p learning --lib facts` fails.
- [ ] **Step 3: Implement.** Add the event variant everywhere `Event` is matched (`kind`, `version`, `payload`). Keep plugins from receiving it exactly the way `group.members_changed` is kept from them (check `plugins/src/manifest.rs` and the delivery code for the catalogue of kinds a plugin may subscribe to; do not add the new kind, and add a test in `plugins` that a manifest naming it is refused if such a test exists for the other internal kind). Emit in `publish_grades` for each row of `published`. Write `facts` and `subscribers()`.
- [ ] **Step 4:** run the new tests, then `cargo test -p learning --lib`, `cargo test -p server --test plugin_events --test gradebook --test submissions --test schedule`. `sqlx prepare`.
- [ ] **Step 5: Commit** `feat(learning): keep content.facts up to date from submissions and published grades (ADR-0093)`.

### Task 5: The access rules read only `content.facts`

**Files:** Modify `content/repo.rs` (`facts`, `mark_done`, `unmark_done`), `content/standing.rs` if it names the old shape. Create `migrations/0052_drop_completions.sql`. Test: existing `server/tests/access_rules.rs` and `server/tests/content.rs`.

**Interfaces:** Consumes Tasks 3 and 4. `repo::facts(conn, course_id, student_id) -> Result<Facts, sqlx::Error>` now selects only from `content.facts` (done = the row exists; scores = `published_score`). `mark_done` upserts `marked_at = now()` keeping the other columns; `unmark_done` sets it NULL and deletes the row when nothing is left.

- [ ] **Step 1: Write the failing tests** in `content/service/tests.rs`: `marking_twice_keeps_the_first_time`, `unmarking_keeps_a_submitted_fact`, `unmarking_with_nothing_else_deletes_the_row`.
- [ ] **Step 2:** fail.
- [ ] **Step 3: Implement** the three repo functions; add `0052_drop_completions.sql` (`DROP TABLE content.completions`; the marks were copied in `0051`); `repo::facts` no longer names `assessment.` or `gradebook.`.
- [ ] **Step 4:** `grep -n "assessment\.\|gradebook\." content/repo.rs` shows no match inside `facts`. Apply the migration, `sqlx prepare`, run `cargo test -p server --test access_rules --test content --test revisions --test courses`. A test that seeds submissions or grades by raw SQL (not through the API) no longer produces facts: change it to go through the API or to call `learning::rebuild_facts`.
- [ ] **Step 5: Commit** `refactor(content): the access rules read content.facts only (ADR-0093)`.

### Task 6: The quiz auto-grade becomes a subscriber

**Files:** Create `migrations/0053_auto_total.sql`, `learning/src/quiz_grade.rs`, `learning/src/quiz_grade/tests.rs`. Modify `assessment/{repo,service,mod}.rs`, `learning/src/lib.rs`.

**Interfaces:**
- Migration: `ALTER TABLE assessment.submissions ADD COLUMN auto_total numeric` (same precision as the grade score columns), set only for a complete auto-grade.
- `assessment` writes `auto_total` in `submit_draft` when `auto.complete`; it no longer calls `gradebook::save_auto_grade`, and `assessment/service.rs` no longer calls `gradebook::` for it.
- `assessment::auto_total(conn: &mut PgConnection, submission_id: Uuid) -> Result<Option<AutoTotal>, AssessmentError>` where `AutoTotal { assignment_id: Uuid, student_id: Uuid, total: Decimal }`; `None` when the attempt has no complete auto-grade.
- `learning::quiz_grade::grade_attempt(conn, event: &Event) -> Result<(), LearningError>`: on `Event::QuizAttemptSubmitted { submission_id, .. }` reads `assessment::auto_total` and calls `gradebook::save_auto_grade`; registered in `learning::subscribers()` for `quiz.attempt_submitted`.

- [ ] **Step 1: Write the failing tests** (`quiz_grade/tests.rs`): `a_complete_quiz_attempt_writes_the_draft_score_and_an_auto_score_history_entry`, `a_quiz_with_a_question_to_grade_by_hand_writes_no_score`, `a_form_submit_writes_no_score`. Existing `server/tests/quiz_and_form.rs` is the regression guard and must pass unchanged.
- [ ] **Step 2:** fail.
- [ ] **Step 3: Implement.** The emit order in `submit_for` stays `SubmissionSubmitted` then `QuizAttemptSubmitted`; both sync subscribers run inside the submit transaction.
- [ ] **Step 4:** apply the migration, `sqlx prepare`, run the new tests and `cargo test -p server --test quiz_and_form --test submissions --test schedule --test plugin_events`.
- [ ] **Step 5: Commit** `refactor(assessment): the quiz auto-grade is a subscriber, not a call into gradebook (ADR-0093)`.

### Task 7: `vumlo-cli facts rebuild` and the demo seed

**Files:** Create `vumlo-cli/src/facts_cmd.rs`. Modify `vumlo-cli/src/main.rs`, `vumlo-cli/src/seed.rs`, `vumlo-cli/Cargo.toml` (`learning.workspace = true`). Test: `vumlo-cli/tests/commands.rs` style.

**Interfaces:** `vumlo-cli facts rebuild [--course <uuid>]` calls `learning::rebuild_facts` in one transaction and writes an audit row (`audit_cli`, action `facts.rebuild`, meta with the course id or `null`), printing how many rows exist after. `seed --demo` calls `learning::rebuild_facts(conn, None)` after it writes grades by SQL.

- [ ] **Step 1: Write the failing tests:** `facts_rebuild_restores_a_deleted_projection_and_writes_an_audit_row`; `seed_demo_leaves_facts_for_its_published_grades`.
- [ ] **Step 2:** fail.
- [ ] **Step 3: Implement** following the shape of an existing small command (`mail_cmd.rs`).
- [ ] **Step 4:** `cargo test -p vumlo-cli --test commands --test seed`; `sqlx prepare`.
- [ ] **Step 5: Commit** `feat(cli): facts rebuild, and the demo seed builds its facts (ADR-0093)`.

### Task 8: Documentation, regeneration and the final checks

**Files:** Modify `docs/decisions/in-progress.md` (ADR-0093: "Built: PR A"), `docs/superpowers/specs/2026-11-12-split-learning-design.md` (migrations are `0051` create+backfill, `0052` drop, `0053` auto_total; the views migration of PR B becomes `0054`), `ARCHITECTURE.md` (the events section: sync subscribers and the registry; data model: `content.facts`, `assessment.submissions.auto_total`; drop `content.completions`; key flows; known limits), `CLAUDE.md` §7 (sync subscribers run in the transaction and abort it on error), `docs/PLAN.md`.

- [ ] **Step 1:** write the doc changes.
- [ ] **Step 2: Checks**, one command at a time: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test -p server --test architecture --test authorization`; `cargo sqlx prepare --workspace --check -- --all-targets`; `SQLX_OFFLINE=true cargo check --workspace --all-targets`; start the server locally, fetch `/api/v1/openapi.json`, `jq -S .` and `diff` it with `web/openapi.json` (no difference).
- [ ] **Step 3: Self-review of the diff** for the user's standing rule (no crooked patches): no `assessment.`/`gradebook.` in `content/repo.rs::facts`; no gradebook call in `assessment/service.rs` for the auto-grade; every `emit` caller path installs a registry.
- [ ] **Step 4: Commit** `docs: events dispatch and content.facts (ADR-0093)`, push `feat/split-learning`, open the PR (base `main`), then stop for review.

---

## Outline of PR B and PR C (own plans, written when PR A is merged)

PR B (boundaries in one crate, migrations from `0054`):
1. Ratchet script and test: lists every cross-schema SQL reference and every `crate::<higher>::` import in `learning`; the list is the work list.
2. `architecture.rs`: one `Unit` per module with layers 7 to 11, `plugins` and `server` moved up, the import scan.
3. Flows replace the upward calls, one commit each: course delete, course publish, course header/people/candidates/cards, module and item delete, assignment edit guards (`EditGuard`), the opened form assignment, the student's result.
4. Views by reader crate (published-only grades view first), `DECLARED_VIEWS` updated, the ratchet list reaches empty.

PR C (the move): `git mv` of the five crates, workspace members, per-crate error/state/router, `server` composition, then CI, `Dockerfile`, `RELEASING.md`, `CLAUDE.md` §4, `ARCHITECTURE.md`, `docs/PLAN.md`; `.sqlx` regeneration; OpenAPI identical.
