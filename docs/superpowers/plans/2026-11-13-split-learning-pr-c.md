# Split `learning` into crates, PR C: the move. Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `teaching`, `content`, `assessment`, `gradebook` become crates of their own under `backend/crates/learning/`, and the crate `learning` keeps only the flows, so Cargo enforces `teaching < content < assessment < gradebook < learning`.

**Architecture:** PRs A, B and the files-usage PR (#61 to #63) made the move mechanical: nothing in a lower module names a higher one or a flow, and every read across modules is a declared view. This PR changes places and names only. Each crate gets its own state (only what it uses), error enum and router; `server` builds the states and merges the routers as it does for `auth` and `org`. The crates leave `learning` bottom-up, one commit each, so every commit builds and `architecture.rs` stays green.

**Tech Stack:** Rust workspace (`backend/`), Cargo path crates, sqlx offline data (`backend/.sqlx`), utoipa/axum routers, GitHub Actions, the release scripts in `.github/release/`.

**Spec:** `docs/superpowers/specs/2026-11-12-split-learning-design.md` (sections 1 to 3 and 8, "PR C"). ADR-0093.

## Global Constraints

- No API, behaviour or screen change: `/api/v1/openapi.json` (`jq -S .`) is identical to `web/openapi.json` after every task that touches a router. No migration.
- No existing test changes an assertion. A test moves with its code; a test that needs a flow moves to the flow crate (task 4, task 5). The number of tests in `cargo test --workspace --all-features -- --list` is the same before and after.
- No shim: no `pub use` of a moved module from `learning`, no `type` alias, no feature flag for the old layout (CLAUDE.md §13). A consumer names the crate it uses and lists it in its `Cargo.toml`.
- Crate names (and workspace dependency keys): `teaching`, `content`, `assessment`, `gradebook`; the flows stay the crate `learning`, in the directory `backend/crates/learning/flows`.
- A crate takes only the dependencies it uses. A lower crate never lists `learning` or a crate above it.
- A `cfg(test)` re-export that another crate's tests need becomes a `testing` feature of the owner, switched on only from `[dev-dependencies]` (`architecture.rs` checks it). Today: `teaching::archive_course`, `content::replace_item_files`, `assessment::replace_submission_files`.
- Unit tests stay in `tests.rs` files (CLAUDE.md §12); a crate's `tests.rs` helper `db_test` installs an empty registry (`events::testing::ensure_installed(events::Subscribers::new)`, as `org` and `plugins` do). A lower-crate test that fails because a subscriber of `learning` is missing belongs in `learning`.
- Change files with Edit/Write, never `sed`/`python`/`perl` through Bash. Move with `git mv` (history). Local runs: `CARGO_BUILD_JOBS=2`, one command at a time; the full workspace and the `dummy` feature are left to CI except where a task says otherwise. After a task that changes a query's file, `cargo sqlx prepare --workspace -- --all-targets --all-features` needs `DATABASE_URL=postgres://vumlo:vumlo@localhost:5432/vumlo` (`docker compose up -d postgres`); a broken build wipes `.sqlx`, restore with `git checkout -- backend/.sqlx`.
- Always check `--workspace --all-targets --all-features` (`ai_agent` is feature-gated and uses these crates).

## Review Focus

1. **The release archive.** `make-archive.sh` removes `crates/*/tests` and the `test-support` lines of `crates/*/Cargo.toml` one level deep; the nested crates would keep both, and the archive would not resolve. Expect: task 1 fixes both globs, and a local run of the script ends in `cargo metadata --locked` without error.
2. **A lower-crate test relied on a subscriber of `learning`** (the quiz grade, the facts). With an empty registry it must fail loudly, not pass vacuously. Expect: each such test moved to `learning`, none deleted.
3. **A wrong `State` extractor.** A route of `content` that asks for `LearningState` compiles inside `learning` today. Expect: each crate's router is bounded by its own state, and `server` has one `FromRef` per state; the OpenAPI diff proves the routes are all mounted.
4. **Offline query data.** Queries in moved files must still be found from the nested crate directories (`backend/.sqlx` is three levels up, as for `modules/ai_agent`). Expect: `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features` and `cargo sqlx prepare --check` clean.
5. **Dependency direction through a state or a type.** `LearningState` must not leak into a lower crate, and a lower crate must not need `learning::subscribers`. Expect: Cargo refuses it; `architecture.rs` still checks schemas, views and the `finish_*` rule.

## File Structure

```
backend/crates/learning/
  flows/    Cargo.toml (package `learning`), src/  (people, groups, reports, courses, items, submissions, files, facts,
            plugin_read, schedule, audit, enrollment, quiz_grade, error, state, tests.rs), tests/ (task 4, task 5)
  teaching/ Cargo.toml, src/{lib.rs, state.rs, model.rs, repo/, routes/, service/, tests.rs}
  content/  Cargo.toml, src/{lib.rs, state.rs, points.rs, document/, types/, model.rs, repo.rs, routes.rs, rules.rs, service.rs, standing.rs, tests.rs}
  assessment/ Cargo.toml, src/{lib.rs, state.rs, model.rs, repo.rs, routes.rs, service.rs, tests.rs}
  gradebook/ Cargo.toml, src/{lib.rs, state.rs, export.rs, layout.rs, model.rs, repo/, routes/, service/, tests.rs}
```

A module's `mod.rs` becomes the crate's `lib.rs` (same `pub use` list, same private `mod` lines). Its `model/tests.rs`-style files keep their relative places.

Dependencies after the move (each crate also uses `access`, `api-types`, `auth`, `axum`, `chrono`, `common`, `events`, `org`, `serde_json`, `sqlx`, `thiserror`, `tokio`, `utoipa`, `utoipa-axum`, `uuid` as the compiler asks, and `vumlo-config` where noted):

| Crate | Also depends on | Needs from the others' tests |
|---|---|---|
| `teaching` | `files` (the file row of a course image, if the compiler asks) | none |
| `content` | `teaching`, `serde`, `tracing`, `vumlo-config`, `files` | `teaching/testing` |
| `assessment` | `content`, `teaching`, `vumlo-config` | `content/testing`, `teaching/testing` |
| `gradebook` | `assessment`, `content`, `teaching`, `rust_xlsxwriter`, `tracing`, `vumlo-config` | `assessment/testing`, `content/testing`, `teaching/testing` |
| `learning` | all four, `files`, `object_store` (dev), `rust_xlsxwriter` only if the compiler asks | all `testing` features, under `[dev-dependencies]` |

State of each crate (`state.rs`, `#[derive(Clone)]`):

- `teaching::TeachingState { pub db: PgPool }`
- `content::ContentState { pub db: PgPool, pub editor: Arc<EffectiveProfiles> }`
- `assessment::AssessmentState { pub db: PgPool }`
- `gradebook::GradebookState { pub db: PgPool, pub grading_scale: GradingScale }`
- `learning::LearningState { pub db, pub config, pub editor, pub files, pub max_file_bytes }` (unchanged; `item_rules()` stays).

`server::AppState` gains a field per new state, with a `FromRef<AppState>` each, built in `AppState::new` from the same pool, config and the one resolved `EffectiveProfiles` (shared by `Arc`).

---

### Task 0: Baseline

**Files:** none (scratchpad only).

- [ ] **Step 1: Branch and build.** Work on `feat/split-learning-crates` (from `main` after #63). From `backend/`: `CARGO_BUILD_JOBS=2 cargo check --workspace --all-targets --all-features`. Expected: finishes without error.
- [ ] **Step 2: Record the baseline** in the scratchpad: `cargo test --workspace --all-features -- --list 2>/dev/null | grep -c ': test$'` into `baseline-test-count.txt`. `web/openapi.json` is the OpenAPI baseline (it equals the server's output on `main`). Expected: a count of some thousand tests.

### Task 1: Move the crate to `learning/flows`

**Files:**
- Move: `backend/crates/learning/{Cargo.toml,src}` → `backend/crates/learning/flows/`
- Modify: `backend/Cargo.toml` (members `"crates/learning/*"` instead of `"crates/learning"`; `learning = { path = "crates/learning/flows" }`), `backend/crates/server/tests/architecture.rs` (the `dir` of every learning unit gets `learning/flows/src…`, `FILE_FINISH_HOMES` the same), `.github/release/make-archive.sh`, `Cargo.lock` (the path moves; no version changes)

**Interfaces:**
- Produces: the layout above with only `flows/` present; later tasks add siblings.

- [ ] **Step 1: `git mv` the crate** (`git mv backend/crates/learning/src backend/crates/learning/flows/src`, same for `Cargo.toml`), edit the manifest and the unit dirs as listed.
- [ ] **Step 2: `make-archive.sh`:** the `rm -rf` list gains `"$stage"/backend/crates/learning/*/tests`; the `sed` that removes the `test-support` lines also runs over `"$stage"/backend/crates/learning/*/Cargo.toml`. Add the same glob to any other `crates/*/` loop in the file; grep the file for `crates/\*` and `crates/modules` to find them.
- [ ] **Step 3: Verify.** `cargo check --workspace --all-targets --all-features` clean; `cargo test -p server --test architecture` passes (12 tests). Expected: both green.
- [ ] **Step 4: Archive dry run.** Commit, then from the repository root `.github/release/make-archive.sh 0.0.0-check <scratchpad>/dist`. Expected: it ends without error (its own `cargo metadata --locked` and the `Cargo.lock` diff check pass). The next tasks re-run it after the last crate leaves (task 6).
- [ ] **Step 5: Commit** `refactor(learning): the flows crate moves to learning/flows (ADR-0093)`.

### Task 2: The crate `teaching`

**Files:**
- Move: `learning/flows/src/teaching/*` → `learning/teaching/src/` (`mod.rs` → `lib.rs`)
- Create: `learning/teaching/Cargo.toml`, `learning/teaching/src/state.rs`, `learning/teaching/src/tests.rs`
- Modify: every file of `flows`, `plugins`, `modules/ai_agent`, `vumlo-cli`, `server` that names `teaching` (compiler lists them); `backend/Cargo.toml` (`teaching = { path = "crates/learning/teaching" }`); `architecture.rs` (`Unit` dir `learning/teaching/src`)

**Interfaces:**
- Produces: `teaching::TeachingState`; `teaching::router<S: Clone + Send + Sync + 'static>() -> OpenApiRouter<S> where TeachingState: FromRef<S>`; `[features] testing = []` exposing `archive_course` (`#[cfg(any(test, feature = "testing"))]`).
- Consumes: nothing from the flows.

- [ ] **Step 1: Failing check.** `cargo test -p server --test architecture` with the `teaching` unit dir changed first fails ("the source tree moved"). Expected: FAIL naming the missing directory.
- [ ] **Step 2: Move and wire.** `git mv`; replace `LearningState` with `TeachingState` in the module's routes (`use crate::LearningState` → `use crate::TeachingState`, the bound `TeachingState: FromRef<S>`); `crate::teaching::model` style paths inside the module become `crate::model`; `crate::tests::db_test` → the crate's own `tests.rs` (`db_test` installing `Subscribers::new`, then `test_support::db_test`); the module's `service/tests.rs` keep their place. The `cfg(test)` export of `archive_course` becomes the `testing` feature.
- [ ] **Step 3: Consumers.** In `flows`, `use crate::teaching` / `crate::teaching::X` becomes `teaching` / `teaching::X`; `LearningError`'s `Teaching` variant keeps `TeachingError`. `learning`'s `Cargo.toml` gains `teaching` and, under `[dev-dependencies]`, `teaching = { workspace = true, features = ["testing"] }`. Same for each other consumer named above. `server`: field `teaching: TeachingState` of `AppState`, a `FromRef`, `teaching::router()` already merged (only its path changes).
- [ ] **Step 4: Verify.** `cargo check --workspace --all-targets --all-features` clean. `cargo test -p teaching` passes (all former `teaching` unit tests: 5 service tests, `model/tests`, `info/tests`). `cargo test -p learning --lib` passes (the former count minus the teaching tests). `cargo test -p server --test architecture` green.
- [ ] **Step 5: Commit** `refactor(teaching): teaching is a crate of its own (ADR-0093)`.

### Task 3: The crate `content`

**Files:**
- Move: `learning/flows/src/content/*` → `learning/content/src/` (`mod.rs` → `lib.rs`); `learning/flows/src/points.rs` + `points/tests.rs` → `learning/content/src/points.rs` (private module; `gradebook` needs it too, see Step 3)
- Create: `learning/content/Cargo.toml`, `src/state.rs`, `src/tests.rs`
- Modify: consumers as in task 2, `backend/Cargo.toml`, `architecture.rs`

**Interfaces:**
- Produces: `content::ContentState`; `content::router`; `content::points::parse_decimal` (`pub mod points`, because `gradebook` and `plugin_read` use it); `testing` feature exposing `replace_item_files`.
- Consumes: `teaching::{CourseTitle, …}` as the module does today (`crate::teaching::X` → `teaching::X`).

- [ ] **Step 1: Move and wire** as in task 2 (`LearningState` → `ContentState`; the route that reads `state.editor` keeps working). `ItemRules`/`LearningState::item_rules` stay where they are: `ItemRules` is a `content` type, `item_rules()` builds it from `learning`'s state.
- [ ] **Step 2: Tests.** `content/service/tests.rs` uses `crate::tests::db_test` only: it moves as it is and `use crate::tests::db_test` points to the crate's helper. Any of its tests that fails because the facts subscribers are not installed moves to `flows` (ledger it).
- [ ] **Step 3: Consumers.** `flows`: `crate::content` → `content`, `crate::points::…` → `content::points::…`; `gradebook` (still in `flows`) too. `LearningState::item_rules` imports from `content`. `server`, `plugins`, `ai_agent`, `vumlo-cli` as the compiler lists.
- [ ] **Step 4: Verify.** `cargo check --workspace --all-targets --all-features` clean; `cargo test -p content` (all former `content` unit tests: 10 service tests, `document`, `model`, `rules`, `types`, `points`); `cargo test -p learning --lib`; `cargo test -p server --test content` and `--test access_rules`; architecture.
- [ ] **Step 5: Commit** `refactor(content): content is a crate of its own (ADR-0093)`.

### Task 4: The crate `assessment`

**Files:**
- Move: `learning/flows/src/assessment/*` → `learning/assessment/src/` except `service/tests.rs`
- Move (as a rewrite of imports only): `assessment/service/tests.rs` → `learning/flows/tests/assessment_locks.rs` (an integration test of `learning`: it uses `submissions::{save_draft, submit}`, `items::{update_item, delete_item}` and `LearningError`, all public)
- Create: `learning/assessment/Cargo.toml`, `src/state.rs`, `src/tests.rs`; `learning/flows/tests/common/mod.rs` only if the file needs `db_test` with the registry installed (`events::testing::ensure_installed(learning::subscribers)`)
- Modify: consumers, `backend/Cargo.toml`, `architecture.rs`

**Interfaces:**
- Produces: `assessment::AssessmentState { db }`, `assessment::router`; `testing` feature exposing `replace_submission_files`.
- Consumes: `content::{Assignment, OnDeadline, …}`, `teaching::TeachingError`.

- [ ] **Step 1: Move the crate** as before. The 11 concurrency tests (`assessment/service/tests.rs`) exercise flows (`submissions::*`, `items::*`) and stay byte-identical in their assertions; they move to `flows/tests/assessment_locks.rs` with `use assessment::…` and `use learning::…` paths. The tests in `assessment/model/tests.rs` stay.
- [ ] **Step 2: Failing check.** Run the moved file before the crate move is complete: `cargo test -p learning --test assessment_locks` fails to compile (missing `assessment` as a dependency). Expected: FAIL, then pass after Step 1 is complete.
- [ ] **Step 3: Consumers** as before (`flows`, `server`, `plugins`, `ai_agent`, `vumlo-cli`).
- [ ] **Step 4: Verify.** `cargo check --workspace --all-targets --all-features` clean; `cargo test -p assessment`; `cargo test -p learning --test assessment_locks` (11 tests); `cargo test -p server --test submissions` and `--test quiz_and_form`; architecture.
- [ ] **Step 5: Commit** `refactor(assessment): assessment is a crate of its own (ADR-0093)`.

### Task 5: The crate `gradebook`

**Files:**
- Move: `learning/flows/src/gradebook/*` → `learning/gradebook/src/` except `service/tests.rs`
- Move: `gradebook/service/tests.rs` → `learning/flows/tests/gradebook_locks.rs` (7 tests; uses `items::*`, `LearningError`)
- Create: `learning/gradebook/Cargo.toml`, `src/state.rs`, `src/tests.rs`
- Modify: consumers, `backend/Cargo.toml`, `architecture.rs`

**Interfaces:**
- Produces: `gradebook::GradebookState { db, grading_scale: GradingScale }`, `gradebook::router`; the routes read `state.grading_scale` where they read `state.config.institution.grading_scale` (gradebook `routes.rs` twice, `routes/groups.rs` seven times).
- Consumes: `content::points::parse_decimal`, `teaching::CardCourse`.

- [ ] **Step 1: Move and wire.** `rust_xlsxwriter` moves to this crate's manifest and leaves `learning`'s.
- [ ] **Step 2: Consumers.** `flows` (`quiz_grade` calls `gradebook::save_auto_grade`, `facts`, `plugin_read`, `courses`, `reports`, …), `server` (`GradebookState` field, `FromRef`), `plugins`, `ai_agent`, `vumlo-cli`.
- [ ] **Step 3: Verify.** `cargo check --workspace --all-targets --all-features` clean; `cargo test -p gradebook`; `cargo test -p learning --test gradebook_locks`; `cargo test -p server --test gradebook`, `--test facts`, `--test schedule`, `--test plugin_events`; architecture.
- [ ] **Step 4: Commit** `refactor(gradebook): gradebook is a crate of its own (ADR-0093)`.

### Task 6: Let Cargo do the direction, tidy `architecture.rs`, check the archive

**Files:**
- Modify: `backend/crates/server/tests/architecture.rs`, `backend/crates/learning/flows/Cargo.toml` (drop unused dependencies), the crate manifests (drop unused)

**Interfaces:** none new.

- [ ] **Step 1: Delete what Cargo now enforces.** Remove `FLOWS`, `crate_paths`, `calls_upward`, the `the_path_scan_sees_grouped_wrapped_nested_and_glob_imports` test and `a_module_calls_only_modules_below_it`: a path into a crate above cannot be written, because the dependency does not exist. Keep the schema rules (`a_module_never_names_the_schema_of_a_module_above_it`, reads through declared views, writes only the own schema), `OWNED`, and `only_the_flows_complete_a_write_that_names_files`. `MODULES` stays if a kept rule uses it. The module-doc header of the file says so.
- [ ] **Step 2: A test that pins the manifest.** `the_four_crates_depend_in_one_direction` in `architecture.rs`: read the four `Cargo.toml`s and fail when a crate lists a crate above it or `learning` (message names the pair). Expected: green; confirm it can fail by adding `gradebook` to `teaching`'s manifest temporarily.
- [ ] **Step 3: Unused dependencies.** For each new manifest and `learning`'s, remove a dependency, check, restore it if the build breaks. `cargo check --workspace --all-targets --all-features` clean.
- [ ] **Step 4: Archive.** Commit, then `.github/release/make-archive.sh 0.0.0-check <scratchpad>/dist`. Expected: no error, and `tar tzf` shows no `learning/*/tests` directory and no `tests.rs`. (The Docker half, `verify-archive.sh`, is left to the release workflow.)
- [ ] **Step 5: Commit** `refactor(learning): Cargo enforces the order of the four crates (ADR-0093)`.

### Task 7: Docs, records, proof

**Files:**
- Modify: `CLAUDE.md` §4 (the layer diagram line for `learning` and the paragraph on modules: the four are crates; the module layout block keeps "`<crate>/src/<module>/`"; `learning` is the flows), `ARCHITECTURE.md` (crates tree line 54, the layer sentence line 83, the table rows 90 to 94, rule 2 of the reads, module layout block), `docs/development.md` (layer chain, "Where code and tests go"), `docs/PLAN.md` (item on the split), `RELEASING.md` (the nested crates in the paragraph on tests), `CHANGELOG.md` (Unreleased: crates, no API change), `docs/decisions/in-progress.md` → `done.md` (ADR-0093 line: "Built (PR C): the separate crates"; remove "Missing"), the spec's status line, `docs/superpowers/plans/2026-11-13-split-learning-pr-c.md` ("Next": org units, ADR-0094)
- Modify: memory `project_roadmap_1_3.md` (item 1 done; next item 2)

- [ ] **Step 1: Write the docs** as listed; `AGENTS.md` is a pointer and needs nothing.
- [ ] **Step 2: Whole proof**, one command at a time, from `backend/`: `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`; the same with `--all-features`; `cargo sqlx prepare --workspace --check -- --all-targets --all-features`; `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`; `cargo test --workspace --all-features -- --list | grep -c ': test$'` equals `baseline-test-count.txt`.
- [ ] **Step 3: Targeted tests** (the rest is CI): `-p teaching`, `-p content`, `-p assessment`, `-p gradebook`, `-p learning`, `-p files`, `-p common`; server tests `architecture`, `authorization`, `access_rules`, `schedule`, `submissions`, `gradebook`, `quiz_and_form`, `plugin_events`, `facts`, `content`, `read_views`.
- [ ] **Step 4: OpenAPI.** `target/debug/vumlo-server --config <scratchpad>/openapi.toml serve --no-worker`, then `curl localhost:18080/api/v1/openapi.json | jq -S .` against `jq -S . web/openapi.json`. Expected: no difference.
- [ ] **Step 5: Commit** `docs: the learning crates (ADR-0093)`. Then one fresh whole-branch review (most capable model), re-grade, one fix pass, push, open the PR (user's standing instruction), and stop for review.

## Self-review

- Spec coverage: five crates and the order (tasks 1 to 5), per-crate state, error and router (tasks 2 to 5, state table), `server` composition (tasks 2 to 5), `architecture.rs` units (tasks 1 to 6), CI paths (no workflow lists `crates/learning`; the archive scripts do, task 1), `Dockerfile` (copies `backend` whole: nothing to change; confirmed by the archive and CI builds), `RELEASING.md`, `CLAUDE.md` §4, `ARCHITECTURE.md`, `docs/PLAN.md` (task 7), `.sqlx` regenerated and checked (task 7), `git mv` for history (all).
- Types: state names and fields are defined once, in the table above.
- Left out on purpose: org units (ADR-0094) and everything after it.
