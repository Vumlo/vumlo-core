# Stage 2 plan: the contract in tiers

Spec: `docs/superpowers/specs/2026-11-26-activity-generalization-design.md` (ADR-0098), stage 2 of 5, section 3. Built on
branch `refactor/activity-tiers`, stacked on `refactor/activity-types-move` (stage 1).

## Context

One trait of eleven required methods serves every type today, so `no_submission` writes seven of them as stubs and a
lecture (stage 4) would write nearly all of them. The contract becomes three tiers, and a type implements the tiers it
has. **No behaviour changes**: the same five types, the same capabilities, the same completion policies, the same API and
the same SQL results. The only visible difference is in the `.sqlx` data (below).

Out of scope: the new hooks (`review`, `responses`, `Held`, plugin views: stage 3), the body hooks and the two new
types (stage 4), `Registrar::activity_type` and `Kind::Missing` (stage 5).

## Design

```rust
pub trait ActivityType: Send + Sync {            // every activity
    fn id(&self) -> &'static str;
    fn completion(&self) -> CompletionPolicy;
    fn graded(&self) -> Option<&dyn Graded>;     // required: every type answers
    fn capabilities(&self) -> Capabilities { .. } // provided, derived from the tiers
}
pub trait Graded: Send + Sync {                  // points, config, a gradebook column
    fn validate_config(..) -> Result<(), ConfigError>;
    fn submissions(&self) -> Option<&dyn Submissions>;       // required
    fn points_total(..) -> Option<Decimal>        { None }   // the four hooks below default to
    fn strip_computed(..)                         {}         // "this type has nothing there"
    fn student_config(..) -> Map<String, Value>   { config.clone() }
    fn structure(..) -> Option<Value>             { None }
    fn config_file_refs(..) -> Result<Vec<FileRef>, ConfigError> { Ok(Vec::new()) }
}
pub trait Submissions: Send + Sync {             // a student hands something in
    fn validate_submission(..); fn claims(..) -> Claims;         // required
    fn submission_file_refs(..) { Ok(Vec::new()) }               // default: names no file
    fn check_file_names(..)     { Ok(()) }
    fn auto_grading(&self) -> Option<&dyn AutoGrading>;          // required
}
pub trait AutoGrading: Send + Sync { fn auto_grade(..) -> Option<AutoGrade>; }   // `None` stays: an unreadable config
```

- `Capabilities` is `{ graded, submissions, auto_graded }`, now **derived** from the tiers (`graded()`,
  `graded()?.submissions()`, `..auto_grading()`), so a type cannot claim what it does not implement. `ids_where` binds it as
  before. Today every type has `graded: true`.
- **`GradedKind`** is new in `activity`: a `Kind` that is known to be graded (`Copy`, equality and `Debug` by id,
  `Deref<Target = dyn Graded>`, `id()`, `completion()`, `capabilities()`, `kind()`). It is `sqlx::Type`, `Decode` and
  `Encode` as `text`, and its decode fails for an id that is registered but not graded. `activity::get_graded(id)` and
  `Types::get_graded(id)` build one; `RegistryError` gains `NotGraded(id)`. `Kind::graded() -> Option<GradedKind>` and
  `Kind::submissions()` are the accessors for a `Kind` of unknown tier.
- **Why a type for it:** `content.assignments` holds graded types only, so the invariant belongs in the type of
  `assignment_type`, not in an `unwrap` at each of about twenty call sites (CLAUDE.md section 10). The call sites keep
  their shape (`assignment_type.validate_config(..)`, `.student_config(..)`) because of the `Deref`.
- A caller that needs the submission tier reads `assignment_type.submissions()` and handles `None` as it handled
  `!capabilities().submissions`: `assessment/src/service.rs:141,479` become `let Some(subs) = .. else { TakesNoSubmissions }`.
  The same for auto grading.

## Steps

### Commit 1: `refactor(activity): the contract is ActivityType, Graded, Submissions and AutoGrading`

1. **Baseline** is the stage-1 result (same machine, same DB): 149 tests in `activity` and `content`, `ai_agent` suite with
   its one known failure. Recount after, and expect `activity` + `content` to change only by the tests added here.
2. `activity/src/model.rs`: the four traits and the derived `Capabilities`; `activity/src/registry.rs`: `GradedKind`,
   `get_graded`, `NotGraded`; `lib.rs` exports.
3. The five types: `impl ActivityType` (id, completion, `graded` returns `Some(self)`), `impl Graded`, and where the type
   takes submissions `impl Submissions`, and where it scores `impl AutoGrading` (only `quiz`; `form` and `file` and
   `text` return `None` from `auto_grading()`). `no_submission` implements only `ActivityType` and `Graded`
   (`submissions()` is `None`) and loses its stubs; the four defaulted `Graded` hooks lose their stubs in every type that
   returned the neutral value.
4. Callers: `content` (`model.rs:261,273,284,587`, `repo.rs` thirteen `assignment_type` casts to `activity::GradedKind`,
   `service.rs` parameters, `routes.rs:336` parses with `get_graded` and maps both errors to
   `ModelError::UnknownAssignmentType`, whose message lists `ids_where(|c| c.graded)`), `gradebook` (`model.rs:387`,
   `repo/work.rs:23,54`), `assessment` (`service.rs:141,239,479,501,558,608-728`), `flows`
   (`plugin_read/repo.rs:48`, `plugin_read/service.rs:84,342`). `content::tests::kind` and `flows::tests::kind` return
   `GradedKind`.
5. Tests: the registry, model and `activity_types.rs` fakes are rewritten to the tiers; the existing per-type tests keep
   running untouched against the moved code; new tests: capabilities are derived (one type per combination), `GradedKind`
   refuses a type that is not graded, `get_graded` errors.
6. `cargo sqlx prepare --workspace -- --all-targets --all-features` **is run once** here, because the casts
   `activity::Kind` to `activity::GradedKind` change the text of about thirteen queries. Run it only on a green build (a
   plain `prepare` on a broken build wipes `.sqlx`; `git checkout backend/.sqlx` restores it), then check the diff is only
   those queries.

### Commit 2: `docs: the contract in tiers (ADR-0098, stage 2)`

`ARCHITECTURE.md` and `CLAUDE.md` sentence on the contract; the ADR-0098 line in `in-progress.md` ("Built: stages 1 and 2");
the ADR-0097 line of `done.md` names the tiers in one clause; spec section 3 gets a note for the three departures
(defaults on four `Graded` hooks, `GradedKind`, `auto_grade` keeps its `Option`).

## Verification

From `backend/`, `CARGO_BUILD_JOBS=2`, one command at a time, dev Postgres up.

1. `cargo test --no-run -p activity -p content -p assessment -p gradebook -p learning -p plugins -p server -p vumlo-cli
   -p vumlo-module-ai-agent`, then `SQLX_OFFLINE=true cargo check --workspace --all-targets --all-features`.
2. `cargo test -p activity -p content -p assessment -p gradebook -p learning`.
3. `cargo test -p server --test activity_types`, `--test architecture`, `--test authorization`, and
   `--features ai_agent --test ai_agent` (same single known failure as the baseline, nothing else).
4. `cargo sqlx prepare --workspace --check -- --all-targets` and with `--all-features` after the one regeneration.
5. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and with `--features dummy`.
6. `git diff --stat` shows nothing under `web/` or `desktop/`.

## Risks

- **`.sqlx` churn** (about thirteen files): expected and reviewed once; it is the price of the invariant living in a type.
- **A graded-only decode** fails a read of an `assignments` row whose type is registered but not graded. No such row can be
  written (creation parses with `get_graded`), and stage 4's migration writes none.
- **Trait-object lifetimes:** `Kind` holds a `&'static dyn ActivityType`, so `graded()` returns `Option<&'static dyn Graded>`
  through it; a type must return `Some(self)` and be a `static`, as the five are.
