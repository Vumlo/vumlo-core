# Stage 1 plan: the built-in activity types move into `activity`

Spec: `docs/superpowers/specs/2026-11-26-activity-generalization-design.md` (ADR-0098), stage 1 of 5. Status: planned,
not started. The code is built on a stacked branch `refactor/activity-types-move`, off `docs/activity-generalization`.

## Context

The five built-in types (`text`, `no_submission`, `file`, `form`, `quiz`, 2,099 lines plus 1,277 lines of tests) sit in
`content/src/types/` although they use nothing of `content` but `ModelError` and `MaxPoints`. Stage 1 moves them into
the crate `activity` so that one crate owns the rules of every type. **No behaviour changes**: same error texts, same
registry contents, same API, same SQL, same `.sqlx` data.

Out of scope (later stages): tiers, new hooks, `lecture` and `shared_file`, any rename of `FormView`, `Holders`, the
`Raw*` structs or the contract, any migration.

## Decisions of this plan (they refine the spec)

1. **`MaxPoints` keeps its name.** The spec said `Points`. Renaming touches about 25 call sites and changes the text of 8
   `.sqlx` queries (`repo.rs:440,674,845,872,895,917,1120,1268` use the override `: MaxPoints`). A move needs neither.
   Spec section 2 is amended in the docs commit.
2. **Export path is `activity::types::<name>`** (`pub mod types`, public submodules `quiz`, `form`, `file`, `text`,
   `no_submission`, and `activity::types::builtin_types()`), as the spec says. `MaxPoints` and `parse_decimal` are
   exported from the crate root. `content` re-exports nothing (CLAUDE.md section 13).
3. **A new `activity::PointsError`** is the error of `MaxPoints::parse`, with the exact text of
   `ModelError::MaxPoints` ("must be a number above 0 and at most 9999.99, with at most two decimals"). `content` adds
   `impl From<PointsError> for ModelError` (to `ModelError::MaxPoints`), so the API text is unchanged.
4. **`FormView` and `decorate_form` stay in `content`** (they take `content::Item`); only their imports change.
   `Holders` is a plain alias (`form.rs:505`) and moves with the form type.

## Mapping of symbols

| Old | New |
|---|---|
| `content::{quiz,form,text}::ID`, `crate::types::{..}::ID` | `activity::types::{quiz,form,text}::ID` |
| `content::quiz_result_questions`, `ResultQuestion` | `activity::types::quiz::{result_questions, ResultQuestion}` |
| `content::Holders`, `types::form::decorate` | `activity::types::form::{Holders, decorate}` |
| `content::builtin_types()` | `activity::types::builtin_types()` |
| `content::MaxPoints`, `content::parse_decimal`, `crate::points` | `activity::MaxPoints`, `activity::parse_decimal` |
| `ModelError::Config(..)` inside a type, `config_error` | `ConfigError::new(..)`, helper deleted |

## Steps

### Commit 1: `refactor(activity): MaxPoints and parse_decimal move from content`

1. **Baseline, measured, before any edit** (from `backend/`, `CARGO_BUILD_JOBS=2`, one command at a time).
   Start dev Postgres first (`docker compose up -d postgres`, `DATABASE_URL=postgres://vumlo:vumlo@localhost:5432/vumlo`),
   because `sqlx prepare --check` needs it. Record in the PR description:
   - `cargo test -p activity -p content -- --list 2>/dev/null | grep -c ': test'` (the sum of both crates);
   - the pass/fail result of every suite of Verification 4 (including the `ai_agent` one: its known failure comes
     from a memory note of 2026-11-14 and may be gone or different now);
   - `cargo sqlx prepare --workspace --check -- --all-targets` **and** the same with `--all-features`; both must pass.
   During the whole move only `--check` is run. A plain `prepare` on a broken build wipes `backend/.sqlx`; if that
   ever happens, `git checkout backend/.sqlx` restores it.
2. New `activity/src/points.rs`: `parse_decimal` (from `content/src/points.rs`), `MaxPoints` with `parse`, `value`,
   `Display`, `sqlx::Type` and `Decode` (from `content/src/model.rs:79-135`), the private `MAX_POINTS_LIMIT`, and
   `PointsError` (`thiserror`, the text above). Tests move with it: `content/src/points/tests.rs` (29 lines) to
   `activity/src/points/tests.rs`, and the `MaxPoints` test at `content/src/model/tests.rs:102` (now asserts
   `Err(PointsError)`).
3. `activity/src/lib.rs`: `mod points; pub use points::{MaxPoints, PointsError, parse_decimal};`.
4. `content`: delete `points.rs` and `points/`; in `model.rs` delete `MaxPoints`, `MAX_POINTS_LIMIT` and the
   imports they alone use; add `impl From<PointsError> for ModelError`; `routes.rs:344` and `:707` map with
   `.map_err(ModelError::from)` (`:707` then keeps `.map_err(ContentError::from)`). `use activity::MaxPoints` in
   `repo.rs`, `service.rs`, `routes.rs` and wherever the compiler asks (`standing.rs:229`, `model.rs:11,91` use
   `activity::parse_decimal`). Remove `pub use points::parse_decimal` and the `MaxPoints` re-export from `lib.rs`.
5. Other crates: `gradebook/src/model.rs:8,119,390` and `service/groups.rs:329`, `flows` tests (`items/tests.rs`,
   `submissions/tests.rs`, `files/service/tests.rs`), `ai_agent/src/tools/authoring.rs:10,387,403-404,440` import
   `activity::{MaxPoints, parse_decimal}`.
6. Check, so that this commit is a safe bisect point: `cargo test --no-run -p activity`, `-p content`, `-p assessment`,
   `-p gradebook`, `-p learning`, `-p plugins`, `-p server`, `-p vumlo-cli`, `-p vumlo-module-ai-agent` (each
   separately: a workspace `check` unifies features and hides missing ones), then `SQLX_OFFLINE=true cargo check
   --workspace --all-targets --all-features` (catches a crate not listed), then both `sqlx prepare --check` runs
   (expected: no diff).

### Commit 2: `refactor(activity): the five built-in types move from content`

1. **Move with history:** `git mv backend/crates/learning/content/src/types backend/crates/learning/activity/src/types`
   (the directory with its `file/`, `form/`, `quiz/`, `registry/` test folders and `tests.rs`).
2. **Manifest:** `activity/Cargo.toml` adds `serde` (with the workspace features, for `Deserialize`) and `chrono`
   (`form.rs` uses `NaiveDate`); description becomes "The contract of an activity type, its built-in types and the
   block document model". `architecture.rs` already allows `serde` and `chrono` (`ACTIVITY_DEPENDENCIES`, line 429).
   `content/Cargo.toml` drops `serde` if `grep` shows nothing left in `content/src` uses it.
3. **Inside the moved files:**
   - `use activity::{..}` becomes `use crate::{..}`; `crate::model::{ModelError, MaxPoints}` becomes `crate::{ConfigError,
     MaxPoints}`.
   - Every `ModelError::Config(x)` becomes `ConfigError::new(x)`; every function that returned
     `Result<_, ModelError>` returns `ConfigError`: the `validate_config` of each type, `text::editor_features`,
     `quiz::file_refs`, and the private `file::parse`/`invalid`, `form::parse` with `rule_*`, `quiz::parse`.
   - `types/mod.rs`: delete `config_error` and every `.map_err(config_error)` (identity now);
     `document_problem` and `shape_problem` return `ConfigError`; `pub(super)` stays as it is.
   - Do not rename the `Raw*` structs: serde's error text can name them (`form.rs:71,80,96`, `quiz.rs:53,66,85`) and
     the tests compare texts.
   - `types/mod.rs` becomes `pub mod file; pub mod form; pub mod no_submission; pub mod quiz; pub mod text;
     mod registry; pub use registry::builtin_types;`. In `activity/src/lib.rs`: `pub mod types;`.
   - Each type file that has no test folder keeps none; `text.rs` and `no_submission.rs` get `types/text/tests.rs` and
     `types/no_submission/tests.rs` (below), declared as `#[cfg(test)] mod tests;`.
4. **Tests that lived in `content` and move:** from `content/src/model/tests.rs` the built-in table loop (121-131), the
   `Text`/`NoSubmission` empty-config tests (141-153) and the `editor_features` tests (370-441, now returning
   `ConfigError`), into the matching `types/*/tests.rs`, **every test verbatim**: none is skipped, merged or
   rewritten, even if it looks like one in `types/registry/tests.rs` (CLAUDE.md section 12; merging duplicates is a
   separate change). **Stay in `content`:** the `UnknownAssignmentType` message test (132-138, needs `install_types()`) and the
   capability test at 366-367, with new imports. The moved tests must not call `activity::install`: `activity`'s own
   `registry/tests.rs:186-192` asserts that nothing is installed, and a global install would break it.
5. **`content` after the move:** delete `types` from `lib.rs` and its re-exports (`lib.rs:36-52`: `builtin_types`,
   `Holders`, `ResultQuestion`, `quiz_result_questions`, the `form`/`quiz`/`text` modules); `service.rs:26,1259,1309`
   import `activity::types::form::{self, Holders}`; `service/tests.rs:115`, `tests.rs:14`, `model/tests.rs:5-7` follow.
6. **Callers** (imports only, per the mapping table): `assessment/src/model.rs:9`, `assessment/src/service.rs:29,581,
   747,775,796`; `flows/src/plugin_read/service.rs:22,94,220,331,355`, `flows/src/items/service.rs:112`, `flows/src/tests.rs:22`
   and the test files named in commit 1; `modules/ai_agent/src/tools/{mod.rs:511,submitting.rs:14,123,authoring.rs:339,364}`;
   `server/src/app.rs:177`, `server/tests/activity_types.rs:93`; `vumlo-cli/src/main.rs:437`, `vumlo-cli/src/seed.rs:264`.
7. **`architecture.rs`:** update the comments only (module doc lines 16-17, unit comment 86-88, doc 426-428) to say
   `activity` also holds the built-in types. No rule changes: the manifest rule, the layer number and
   `no_builtin_type_name_is_left` are unaffected.

### Commit 3: `docs: where the built-in types live (ADR-0098, stage 1)`

- `CLAUDE.md` section 4, `activity` paragraph, and `ARCHITECTURE.md:60` and `:506`: the types are in `activity`.
- `docs/decisions/done.md:82` (ADR-0097): annotate "the five built-in types moved into `activity` (ADR-0098, stage 1)",
  in the file's habit (see line 69).
- `docs/decisions/planned.md`: move the ADR-0098 line to `in-progress.md` as "Built: stage 1 (the types and
  `MaxPoints` live in `activity`). Missing: stages 2 to 5."
- Spec section 2: `MaxPoints` keeps its name, `PointsError` is new.
- The historical spec `2026-11-14-activity-types-design.md` is left as written.

## Verification

All from `backend/`, `CARGO_BUILD_JOBS=2`, one command at a time (an OOM crash happened once). Dev Postgres:
`docker compose up -d postgres`, `DATABASE_URL=postgres://vumlo:vumlo@localhost:5432/vumlo`.

1. **No test is lost or added:** the summed count of `cargo test -p activity -p content -- --list` after the move equals
   the baseline sum exactly (tests only change crate). Any difference is a bug in the move.
2. **Per-crate compile, features not unified:** `cargo test --no-run -p activity`, `-p content`, `-p assessment`,
   `-p gradebook`, `-p learning`, `-p plugins`, `-p server`, `-p vumlo-cli`, and `-p server --features ai_agent`.
3. `cargo test -p activity`, `-p content` (the moved tests run in `activity`).
4. Database suites that name the types, each alone, compared with the baseline result: `cargo test -p server --test
   activity_types`, `--test architecture`, `--test authorization`, and `--features ai_agent --test ai_agent`. A
   failure that existed in the baseline must be the same failure; any other difference is a bug in the move.
5. `cargo sqlx prepare --workspace --check -- --all-targets` and the same with `--all-features` (the `ai_agent` queries
   exist only there), and `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`: **no diff in
   `backend/.sqlx/`.** A diff means a query text changed, which a move must not do.
6. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and the same three with
   `--features dummy`. CI then runs the full suite and `--test-threads=16`.
7. No API shape changes: `git diff --stat` shows nothing under `web/` or `desktop/`.

## Risks

- **Error text drift.** `ModelError::Config` and `ConfigError` both print `{0}`, and `PointsError` copies the
  `MaxPoints` text. The moved tests compare `to_string()`; a changed message fails them.
- **Global registry in tests** (see step 4 of commit 2).
- **`content` loses its `serde` dependency** only if nothing else there derives; verify with the compiler, not by grep.
- **Stacking.** The branch is based on `docs/activity-generalization`, so the PR carries the spec commit until that
  branch merges first.
