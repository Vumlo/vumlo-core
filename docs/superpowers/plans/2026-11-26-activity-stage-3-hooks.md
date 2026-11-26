# Stage 3 plan: the features of a type are hooks of the contract

Spec: `docs/superpowers/specs/2026-11-26-activity-generalization-design.md` (ADR-0098), stage 3 of 5, section 5. Built on
branch `refactor/activity-hooks`, stacked on `refactor/activity-tiers` (stage 2).

## Context

After stages 1 and 2 the rules of every type live in `activity`, but the features a type owns are still served by other
crates that compare the type id: the quiz result and the quiz event (`assessment`), the form responses and the "taken"
overlay of a form (`assessment`, `content`, `flows`), and the plugin reads of a form's fields and answers and of a quiz
(`flows`, `plugins`, `access`, `events`). Stage 3 moves each decision behind the contract, so that **no crate outside
`activity` compares a built-in type id** (modules, `vumlo-cli` and tests may name a built-in).

Two commits, because the first changes nothing a user or a plugin can see and the second is the one deliberate break.

## Design (refined from the spec)

The spec sketched data hooks (`review`, `responses`, `Held`). Reading the code showed that two of them are simpler:

- **Responses are already generic.** `assessment::responses` lists every current student's latest attempt and who has none;
  only the gate `id != form::ID` names a type. So the hook is a bool, `Submissions::lists_responses(&self) -> bool`
  (default `false`; `form` is `true`), not a data hook.
- **The overlay is generic too.** `assessment::form_facts` already computes the facts from `claims()` of any type. They move
  to `activity` as `ClassFacts { answered, holders }` (with the `Holders` alias), and the hook is
  `Graded::annotate_config(&self, config: &mut Map, facts: &ClassFacts, viewer: Viewer)` (default: nothing), with
  `Graded::reads_class_facts(&self) -> bool` (default `false`) telling `content` whether to ask for them. `form` implements
  both with today's `decorate`. `Viewer { staff, user }` replaces `content::FormView`.
- **A review is a tier**, like auto grading: `Submissions::reviewing(&self) -> Option<&dyn Reviewing>` with
  `Reviewing::review(config, content, auto_scores) -> Option<Vec<ReviewQuestion>>`. `ReviewQuestion` is today's
  `ResultQuestion` (id, points, got, answer, correct, explanation), moved to the root of `activity`. `quiz` is the only
  type that reviews.
- **Plugin views:** `Graded::plugin_config(&self, config) -> Option<Map<String, Value>>` (default `None`; `form` and `quiz`
  return `student_config(config)`: the fields of a form, the questions of a quiz without the key) and
  `Submissions::plugin_answers(&self, content) -> Option<Map<String, Value>>` (default `None`; `form` returns its `answers`).
- The lock of the claims check is named `claims:<assignment id>` instead of `form:<..>`.
- The API shape of the web app does not change in this stage: `GET /assignments/{id}/result` and `/responses` keep their
  paths and DTOs (`QuizResult`, `QuizResultQuestion` keep their names; renaming them touches the web app and belongs to
  stage 4 with `Item.type`).

## Commit 1: `refactor(activity): the features of a type are hooks of the contract` (no visible change)

1. `activity`: `contract.rs` gets `Viewer`, `ClassFacts`, `Holders` (moved from `types/form.rs`), `ReviewQuestion` (moved
   from `types/quiz.rs`); `model.rs` gets `Reviewing`, `Submissions::{reviewing, lists_responses}`,
   `Graded::{reads_class_facts, annotate_config}`; `form` implements `lists_responses`, `reads_class_facts`,
   `annotate_config` (its `decorate` becomes the body); `quiz` implements `Reviewing`.
2. `assessment`: `begin_quiz_result` / `finish_quiz_result` become `begin_result` / `finish_result`, gated by
   `submissions(kind)?.reviewing()` instead of `id() != quiz::ID`, and `QuizResult` becomes `AttemptResult` (internal; the
   flow maps it to the same DTO); `responses` is gated by `lists_responses()`; `form_facts` becomes `class_facts` and
   returns `activity::ClassFacts`; `FormFacts` is deleted.
3. `content`: `ItemRead.form: Option<FormView>` becomes `ItemRead.viewer: Option<Viewer>`, set when
   `kind.reads_class_facts()`; `decorate_form` becomes `annotate_item(item, viewer, facts)` calling the hook;
   `FormView` is deleted.
4. `flows`: `items::get` and `items::result` follow the new names.
5. Tests move with their code; new tests: each default hook (`reads_class_facts`, `lists_responses`, `reviewing`) is off for
   the types that do not implement it and on for `form` and `quiz` (one table test over the five, next to the capability
   table), and `annotate_config` leaves a type with no override unchanged.

## Commit 2: `feat(plugins)!: the plugin protocol names no type (api 2)`

The one deliberate break. Nothing runs against it today (owner, 2026-11-26).

1. Scope `quizzes:read` becomes `attempts:read` (`access/src/plugin.rs`: the variant `AttemptsRead`, its string, the list).
2. Event `quiz.attempt_submitted` becomes `attempt.autograded`: `events::Event::AttemptAutograded`, `plugins::SUBSCRIBABLE`,
   and `assessment` emits it when the type has an `auto_grading()` tier. Comments and the migration text that name the old
   event are history and stay.
3. Routes: `GET /plugin-api/assignments/{id}/quiz` and `PluginQuiz` are removed (the questions come through
   `plugin_config` in the assignments list); `GET /plugin-api/assignments/{id}/attempts` works for any auto-graded type
   under `attempts:read`; `GET /plugin-api/submissions/{id}/fields` becomes `/answers` with `PluginAnswers`
   (was `PluginFormAnswers`), answered by `plugin_answers`; `PluginAssignment.fields` becomes `config`.
4. `plugins::manifest`: `API_VERSION` `"1"` becomes `"2"`; the literals `"api": "1"` in about ten test files, in
   `vumlo-cli/tests`, and in `examples/hello-plugin` become `"2"`.
5. `flows::plugin_read` loses `form::ID` and `quiz::ID`; `web/openapi.json` and `web/src/lib/api/schema.d.ts` are
   regenerated (descriptions and the three routes only).
6. Not part of this stage: the Python SDK and the catalogue in `vumlo-plugins` (separate repository; brought to `api = "2"` as
   its own piece of work).

## Commit 3: `docs: the features of a type are hooks (ADR-0098, stage 3)`

`ARCHITECTURE.md` (plugin section: scope, event, routes, `api = "2"`), `CHANGELOG.md` (an upgrade note: the plugin protocol
is `api = "2"`, with the renames), the ADR-0098 line ("Built: stages 1 to 3"), spec sections 3 and 5 (the bool and the
`ClassFacts` as built), `examples/hello-plugin/README.md` if it names the old scope or routes.

## Verification

From `backend/`, `CARGO_BUILD_JOBS=2`, one command at a time. The baseline is the stage-2 result: 152 tests in `activity` plus
`content`, the DB suites green except the one known `ai_agent` failure.

1. After every commit: `cargo test --no-run` for each crate of the workspace (`activity content assessment gradebook
   learning plugins server vumlo-cli vumlo-module-ai-agent`), then `SQLX_OFFLINE=true cargo check --workspace --all-targets
   --all-features`.
2. Commit 1: the test count of `activity` plus `content` is 152 plus the new tests only; `.sqlx` has **no** diff; the suites
   `quiz_and_form`, `submissions`, `activity_types`, `authorization` and `ai_agent` give the baseline results.
3. Commit 2: the suites `plugin_reads`, `plugin_events`, `plugin_mail`, `authorization`, `schedule`, `submissions` and
   `quiz_and_form` pass with the renamed names; `vumlo-cli` `plugins` and `doctor` tests pass with `api = "2"`.
4. A grep outside `activity`, `vumlo-cli`, `modules`, tests and migrations for `form::ID`, `quiz::ID`, `text::ID`,
   `no_submission::ID`, `file::ID` finds nothing. (Enforced by `architecture.rs` in stage 5; checked by hand here.)
5. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and with `--features dummy`;
   `sqlx prepare --check` with and without `--all-features`.

## Risks

- **Breadth of commit 2.** Many server tests spell the old scope, event, routes and `api`. They are renamed, never loosened.
- **`sqlx prepare`:** commit 1 changes no query; commit 2 changes none either (the event type is a string at the database).
  Any `.sqlx` diff is a surprise to investigate.
- **The plugin SDK** is out of step until its own work lands; no plugin is compatible today.
