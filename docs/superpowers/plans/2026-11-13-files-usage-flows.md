# The file-usage check becomes part of the flows. Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `content`, `assessment` and the gradebook export stop calling `learning::files`, so the four modules are independent of every flow and the crate split (PR C) has no cycle, while a file can still be named only by an owner that may use it, with the check and the attach atomic.

**Architecture:** The check that "this file may be named by this owner" is policy that reads `content` and `assessment`, so it is a flow (`learning::files::check`). Each write that names files is split in the owner into `begin_*` (validate, take the owner's locks, list the files named) and `finish_*` (raise what a `FileVerdict` says, write, attach), and a flow in `learning` runs `begin`, `check`, `finish` on one connection. `check` first locks the named file rows `FOR UPDATE` in id order, then reads the references, so the attach that follows in the same transaction cannot race another one. `files` (the crate) stays below everything: it gets the lock, an `is_image` helper and the plain `FileVerdict` data type, nothing about owners.

**Tech Stack:** Rust, sqlx, axum, utoipa. One new flow module (`learning::submissions`), no migration, no API change.

**Spec:** `docs/superpowers/specs/2026-11-12-split-learning-design.md` (sections 3 to 5); `docs/superpowers/plans/2026-11-13-split-learning-pr-b2.md` ("Next"). ADR-0093. Branch `feat/split-files-usage`, stacked on `feat/split-boundaries` (PR #62); its own PR, before PR C.

## Global Constraints

- Run from `backend/`; targeted runs, `CARGO_BUILD_JOBS=2`, one command at a time; the full workspace and `--features dummy` runs are left to CI. Check `--workspace --all-targets --all-features` (the `ai_agent` module is feature-gated). `cargo sqlx prepare --workspace -- --all-targets --all-features`.
- The API is unchanged: `/api/v1/openapi.json` (`jq -S .`) stays identical to `web/openapi.json`; no existing test changes an assertion except to call the flow instead of the module.
- The four rules of `ensure_usable` hold exactly: (1) the owner already holds the file (whoever else holds it too); (2) nobody references it and the actor uploaded it; (3) for a submission only: every row that holds it is an attempt or draft of the same student on the same assignment; (4) anything else, an unknown id included, is `Unusable`. Their tests move to `check` unchanged.
- Order of locks, everywhere: the owner's rows (module, assignment, draft) first, then file rows in ascending id, then the reference rows (written by `finish_*`). No other lock is taken while file rows are held.
- No trait, callback or injected closure to hide the dependency (user's rule). `files` (crate) knows nothing of `content` or `assessment`.
- Nothing is a shim: delete `ensure_usable`, `first_non_image` and the old whole-write functions (`content::create_item`, `create_file_item`, `assessment::save_draft`, `submit`, `auto_submit`) instead of keeping them beside the flows.
- Services take `&mut PgConnection`, never begin or commit; the handler opens the transaction (CLAUDE.md section 4). Conventional Commits.

## Review Focus

- Two transactions naming one unreferenced upload (two items, or an item and a draft): exactly one wins, the other is refused as `Unusable` after it waited.
- Two flows naming the same files in opposite orders do not deadlock.
- The sweeper cannot delete a file that a running attach has locked or that a committed attach now references.
- The scheduler's auto-submit is checked like a student's submit: a draft whose file is gone is refused, not submitted with a dangling reference.
- A request that is wrong in two ways (an unusable file and, say, a bad config) now reports the validation error first and the file second; each is still reported with its field path.

## Files

- `files` crate: `src/repo.rs`, `src/service.rs`, `src/lib.rs`, `src/mime.rs` (`is_image`), new `src/verdict.rs`.
- `common`: `content_disposition` (moved from `learning::files::routes::disposition`).
- `learning::files`: `service.rs` (`check` replaces `ensure_usable` and `first_non_image`), `sweep.rs`, `mod.rs`, `routes.rs`.
- `learning::content`: `service.rs`, `routes.rs`, `mod.rs`, `model.rs`; `learning::assessment`: `service.rs`, `routes.rs`, `mod.rs`.
- `learning::items`: gains `create_item`, `create_file_item`, `update_item` (extended); new `learning::submissions` (`mod.rs`, `service.rs`, `routes.rs`); `learning::schedule/mod.rs`, `lib.rs`, `server/src/app.rs`.
- `modules/ai_agent`: `tools/authoring.rs`, `tools/submitting.rs`.
- Tests: `files/src/service/tests.rs`, `learning/src/files/service/tests.rs`, `server/tests/files.rs`, `server/tests/architecture.rs`, and the unit-test helpers that call the removed functions (`content/service/tests.rs`, `assessment/service/tests.rs`, `gradebook/service/tests.rs`).
- Docs: `ARCHITECTURE.md`, `CLAUDE.md` section 4, `CHANGELOG.md`, `docs/decisions/in-progress.md`, the B2 plan's "Next".

---

### Task 1: `files` crate: the lock, `is_image`, `FileVerdict`

**Files:** `files/src/repo.rs`, `service.rs`, `mime.rs`, `lib.rs`, new `verdict.rs`; tests in `files/src/service/tests.rs`.

**Interfaces:**
- Produces `files::lock(conn: &mut PgConnection, ids: &[Uuid]) -> Result<Vec<File>, FilesError>`: the rows that exist, `ORDER BY id`, `FOR UPDATE`, held until the transaction ends; an unknown id is simply absent.
- Produces `files::try_lock(conn, id: Uuid) -> Result<Option<File>, FilesError>`: `FOR UPDATE SKIP LOCKED`; `None` when absent or locked.
- Produces `files::is_image(mime: &str) -> bool` (PNG, JPEG, GIF, WebP), next to `is_inline`.
- Produces `files::FileVerdict { pub unusable: Option<Uuid>, pub not_image: Option<Uuid>, pub names: BTreeMap<Uuid, String> }` with `Default`: plain data, the answer of the check; `names` holds the name of every named file that exists.
- Produces `files::forget(conn, id)` (the row only) and `files::drop_object(storage, id)` (the stored object only); `files::delete` is the two in order.

- [ ] **Step 1: Write the failing tests** (`files/src/service/tests.rs`, `db_test`): `lock_returns_the_rows_that_exist_in_id_order`, `a_second_lock_waits_until_the_first_transaction_ends` (second `lock` on a spawned task, `test_support::wait_until_blocked`, then commit the first and join), `try_lock_skips_a_locked_row`, `is_image_accepts_png_jpeg_gif_and_webp_only`.
- [ ] **Step 2:** `cargo test -p files` fails to compile (functions missing).
- [ ] **Step 3:** Implement the signatures above; `lock` is one statement with `WHERE id = ANY($1) ORDER BY id FOR UPDATE`.
- [ ] **Step 4:** `cargo test -p files` passes; `cargo clippy -p files --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `feat(files): lock file rows, is_image and the verdict type (ADR-0093)`.

### Task 2: the sweeper takes the same lock

**Files:** `learning/src/files/sweep.rs`; tests in `learning/src/files/service/tests.rs`.

**Interfaces:** Consumes `files::try_lock`, `files::forget`, `files::drop_object`. Produces no new public item. Removes the `ponytail:` comment about the race.

- [ ] **Step 1: Write the failing tests**: `sweep_leaves_a_file_that_a_running_attach_has_locked` (hold `files::lock` in an open transaction on an old unreferenced file, `sweep`, assert the row and object are kept), `sweep_rechecks_the_references_under_the_lock` (a reference row inserted and committed after the candidate list was read: kept).
- [ ] **Step 2:** the first fails (today the sweep deletes it).
- [ ] **Step 3:** `delete_if_orphan` opens a short transaction: `try_lock`; `None` means skip; `is_referenced`; `forget`; commit; then `drop_object` outside the transaction (the object is removed only after the row is gone for good, as today).
- [ ] **Step 4:** both tests and the existing `sweep_*` tests pass.
- [ ] **Step 5: Commit** `fix(files): the orphan sweep respects the file lock (ADR-0093)`.

### Task 3: `learning::files::check` replaces `ensure_usable` and `first_non_image`

**Files:** `learning/src/files/service.rs`, `repo.rs`, `mod.rs`; tests in `learning/src/files/service/tests.rs`.

**Interfaces:**
- Produces `learning::files::check(conn: &mut PgConnection, user_id: Uuid, owner: &UsageOwner, refs: &[content::FileRef]) -> Result<files::FileVerdict, FileError>`. Order: `files::lock` on the distinct ids; read the references (the declared views, as today); apply rules 1 to 4 in the order of `refs`, the first failing id goes to `unusable`; the first image-flagged ref whose locked row is not an image goes to `not_image`; `names` from the locked rows. `UsageOwner` is unchanged.
- Removes `ensure_usable`, `first_non_image`, `IMAGE_MIMES`. `FileError` loses nothing (it still serves upload and read).
- The references stay read through the views `content.item_file_list`, `assessment.submission_file_list` and `assessment.submission_index` (the declared interface of B2); no second interface is added beside them.

- [ ] **Step 1:** re-point the existing tests (`ensure_usable_*`, the resubmission tests, `a_file_held_by_an_item_is_not_usable_by_a_draft_even_next_to_an_own_attempt`) to `check` and assert `verdict.unusable`; add `check_names_the_first_unusable_file_in_the_order_given`, `check_names_a_non_image_and_returns_every_name`.
- [ ] **Step 2:** fails to compile (`check` missing).
- [ ] **Step 3:** implement `check`; keep `held_by_own_attempts` as is.
- [ ] **Step 4:** the flow tests pass.
- [ ] **Step 5: Commit** `refactor(learning): the file-usage check is a verdict computed under the file locks (ADR-0093)`.

### Task 4: items: `begin_` / `finish_` for create and update, and the flows

**Files:** `content/service.rs`, `routes.rs`, `mod.rs`, `model.rs`; `items/{service,routes,mod}.rs`; `server/src/app.rs`; `modules/ai_agent/src/tools/authoring.rs`; the content, files and gradebook unit-test helpers.

**Interfaces:**
- Produces `content::begin_create_item(conn, actor, rules, module_id, request: NewItemInput) -> Result<ItemCreation, ContentError>` and `content::finish_create_item(conn, actor, creation, files: &FileVerdict) -> Result<Item, ContentError>`. `NewItemInput { title: ContentTitle, kind: ItemKind, body: Option<Value>, assignment: Option<Assignment> }` replaces the pair `create_item` / `create_file_item`. `ItemCreation::files(&self) -> &[FileRef]` lists what is named. `finish` raises `FileUnusable { id, field }` or `NotAnImage { field }` from the verdict (field from the matching `FileRef.path`), then inserts and attaches.
- `ItemUpdate::files(&self) -> &[FileRef]` (body and config); `finish_update_item(conn, actor, update, facts: AssignmentFacts, files: &FileVerdict)` raises the same two first, before the other guards. `set_due_at` passes `FileVerdict::default()` (it names no file).
- Produces `content::new_item(request: CreateItemRequest) -> Result<NewItemInput, ApiError>` and `content::under_assignment(error: ContentError) -> ApiError` (the request mapping and the error path rewrite that the create handler used; `pub` like `api_item`).
- Produces `items::create_item(conn, actor, rules, module_id, input: NewItemInput) -> Result<Item, LearningError>` (begin, `files::check` with `UsageOwner::Item(creation.id())`, finish) and extends `items::update_item` the same way. `POST /modules/{id}/items` moves to `items::routes` (same path, annotation, `operation_id`).
- Removes `ContentError::Files`, `usable_files`, `content::create_item`, `content::create_file_item` and the `crate::files` import.

- [ ] **Step 1: Write the failing tests.** Learning unit tests: `an_item_naming_a_file_of_another_owner_is_refused_with_its_field_path` through `items::create_item` and `items::update_item` (body and config), `an_image_block_needs_an_image_file`. In `server/tests/files.rs` (HTTP): `two_items_cannot_both_take_one_upload` (two transactions via the test pool, the second blocked with `wait_until_blocked`, then `422`/`Unusable` after the first commits), `an_item_and_a_draft_cannot_both_take_one_upload`, `two_updates_naming_the_same_files_in_opposite_orders_do_not_deadlock` (two items that already hold both files).
- [ ] **Step 2:** the unit tests fail to compile; the HTTP race test fails today (both succeed).
- [ ] **Step 3:** implement the interfaces; `begin_create_item` is today's `create` up to and including `item_file_refs`, `finish_create_item` the rest; `begin_update_item` stops calling `usable_files` and keeps the refs it computed. Switch the handler, `ai_agent` authoring (3 calls) and the test helpers to the flows; content's own unit tests use `begin_` / `finish_` with `FileVerdict::default()` only for items that name no file.
- [ ] **Step 4:** `cargo test -p server --test files --test content --test access_rules --test quiz_and_form`, `cargo test -p learning --lib`; OpenAPI identical.
- [ ] **Step 5: Commit** `refactor(learning): creating and updating items check their files in a flow (ADR-0093)`.

### Task 5: submissions: `begin_` / `finish_` for save and submit, the `submissions` flow, the scheduler

**Files:** `assessment/service.rs`, `routes.rs`, `mod.rs`; new `submissions/{mod,service,routes}.rs`; `lib.rs`, `server/src/app.rs`, `schedule/mod.rs`; `modules/ai_agent/src/tools/submitting.rs`; the assessment and files unit tests.

**Interfaces:**
- Produces `assessment::begin_save_draft(conn, actor, assignment_id, content, profiles, now) -> Result<DraftSave, AssessmentError>` (everything up to and including the draft row lock and the id it will keep), `DraftSave::files(&self) -> &[FileRef]`, `DraftSave::owner(&self) -> UsageOwner`-equivalent data (`submission_id`, `assignment_id`), and `assessment::finish_save_draft(conn, actor, save, files: &FileVerdict) -> Result<Draft, AssessmentError>`.
- Produces `assessment::begin_submit(conn, actor, assignment_id, profiles, now)` and `assessment::begin_auto_submit(conn, due: &DueDraft, profiles, now)`, both returning `SubmitStart` (the lock order of `submit_for` kept: assignment `FOR SHARE`, draft `FOR UPDATE`, attempt count, validation, claims), with `files()`; one `assessment::finish_submit(conn, start, files: &FileVerdict) -> Result<Attempt, AssessmentError>` (verdict, auto-grade, `submit_draft`, attach, emit).
- `finish_*` map the verdict to the errors `check_files` gave: `unusable(path)` for `unusable`, the image message for `not_image`, and the file-name rule through `content::check_file_names(config, body, &verdict.names)` for a `file` hand-in.
- Produces `learning::submissions::{save_draft, submit, auto_submit}` (begin, `files::check` with `UsageOwner::Submission { id, assignment_id }`, finish) returning `Result<_, LearningError>`; the two routes (`PUT /assignments/{id}/submission/draft`, `POST /assignments/{id}/submission`, confirm the exact paths) move to `submissions::routes`; `assessment` exports `api_draft` and `api_attempt` for them. `schedule` calls `submissions::auto_submit`.
- Removes `check_files`, `assessment::save_draft`, `submit`, `auto_submit`, the `crate::files` import and the `FileError` mapping in `assessment`.

- [ ] **Step 1: Write the failing tests.** `a_draft_cannot_take_a_file_another_student_just_attached` (race, as in task 4), `an_auto_submit_whose_file_is_gone_is_refused_and_leaves_the_draft` (delete the file row after the save; `auto_submit` returns an error with `refuses_submit()` true; the draft is kept), `a_submit_checks_the_files_of_the_draft_again`. Re-point the resubmission tests to `submissions::save_draft`.
- [ ] **Step 2:** compile failure / the race test fails.
- [ ] **Step 3:** implement; keep `AssessmentError::refuses_submit` semantics by mapping an unusable verdict to the same `Document` error as today.
- [ ] **Step 4:** `cargo test -p server --test submissions --test files --test quiz_and_form --test schedule --test plugin_events`, `cargo test -p learning --lib`; OpenAPI identical.
- [ ] **Step 5: Commit** `refactor(learning): saving and submitting check their files in a flow (ADR-0093)`.

### Task 6: the last flow dependency, the guard and the docs

**Files:** `common`, `learning/src/files/routes.rs`, `gradebook/routes.rs`, `server/tests/architecture.rs`, docs.

**Interfaces:** Produces `common::content_disposition(name: &str, inline: bool) -> HeaderValue` (moved; `files` routes and the gradebook export use it). Deletes `RATCHET_FLOWS` and its stale test: `a_module_calls_only_modules_below_it` is strict.

- [ ] **Step 1: Write the failing test** in `architecture.rs`: `only_the_flows_complete_a_write_that_names_files` scans non-test code outside `learning/src/{items,submissions}` and the defining module for `finish_create_item`, `finish_update_item`, `finish_save_draft`, `finish_submit` (allowed: their own module for `set_due_at`, and `tests.rs`), so no entry point can skip `check`. It fails if you add a stray caller; first run it red by calling one from `plugins`.
- [ ] **Step 2:** red as above; then implement the move of `disposition` and empty `RATCHET_FLOWS`.
- [ ] **Step 3: Docs.** ARCHITECTURE.md rule 2 (the exception is gone; the order of locks owner rows, then file rows ascending, then references; the `check` / `begin_` / `finish_` shape; the new flow `submissions`), CLAUDE.md section 4, CHANGELOG ("No change to the API"), the ADR-0093 line, the B2 plan's "Next".
- [ ] **Step 4: Checks:** `cargo fmt --all --check`; clippy `--workspace --all-targets` with and without `--all-features`, `-D warnings`; `cargo sqlx prepare --workspace --check -- --all-targets --all-features`; `SQLX_OFFLINE=true cargo check --workspace --all-targets`; targeted tests of tasks 4 and 5 once more; OpenAPI identical (server with the scratch config, `curl`, `jq -S`, `diff`).
- [ ] **Step 5:** self-review of the diff (no leftover `ensure_usable`, `first_non_image`, `ContentError::Files`; no `crate::files` in the four modules), one fresh whole-branch review, push, PR with base `feat/split-boundaries` until #62 merges, then stop for review.
- [ ] **Step 6: Commit** `refactor(learning): no module names a flow any more (ADR-0093)`.

---

## Self-review

- **Requirements:** (1) check and attach in one transaction: `begin`, `check` and `finish` run on the connection of the handler's transaction (tasks 4, 5; the race tests prove it). (2) `FOR UPDATE`, ascending id, before reading references: `files::lock` (task 1), used first in `check` (task 3); the sweeper uses it too (task 2); a reversed-order test guards deadlocks. (3) Every caller: the two HTTP handlers of each family, `ai_agent` (authoring x3, submitting x2), the scheduler, test helpers; the guard test in task 6 makes a bypass fail the build. (4) The four rules: the existing tests move to `check` unchanged (task 3). (5) No trait or shim: plain `FileVerdict` data passed to `finish_*`.
- **Type consistency:** `FileVerdict` (task 1) is what `check` returns (task 3) and what every `finish_*` takes (tasks 4, 5); `FileRef` is the existing `content::FileRef`; `UsageOwner` unchanged.
- **Scope:** about 6 tasks, none a migration. The largest are 4 and 5; each ends green and independently testable.
- **Decision recorded:** `FileVerdict` lives in the `files` crate because both `content` and `assessment` must accept it and neither may import the flow; it carries data only. The reference reads stay on the declared views, the one interface B2 created.
