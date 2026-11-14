# Activity types: the contract

Status: design, not built. ADR-0097. Delivery: one PR together with the web part of org units (ADR-0094, PR B); the
plan is `docs/superpowers/plans/2026-11-14-units-web-and-activity-types.md`. Roadmap item 3.

## 1. Intent

The kind of an assignment (`text`, `no_submission`, `quiz`, `form`, `file`) is a closed enum, `content::model::BuiltinType`,
and many places decide on it: string literals in SQL, `== BuiltinType::File` in a service, `"form"` and `"quiz"` constants
in the plugin read layer, two migration CHECKs. A new type (question bank, H5P, SCORM, LTI, a module's own) cannot be
added without editing all of them. ADR-0008 named the trait `AssignmentType` "the future plugin boundary"; this makes it
true.

Deliver:

1. A contract, `ActivityType`: what a type is (id), what it can do (capabilities), how it is validated, graded and claims
   things (the nine methods of `AssignmentType`), and when its work counts as done (a completion policy).
2. A registry the composition root fills once, like `events::install`. Code asks the registry, never matches on a name.
3. The five built-in types written against the contract, with no change in behaviour.
4. Every decision that today names a type asks a capability or the contract instead.

Non-goals: item kinds (`text`, `assignment`, `file`) stay as they are (ADR-0024, a DB CHECK); a type that comes from a
module (the edge `module-api -> activity` arrives with the first module that provides one, items 5 and 6); per-activity
completion policies and the events `activity.completed` and `course.completed` (roadmap item 4); telling the web app
the capabilities (its per-type pages and `no_submission` filters stay for now); a change to any API shape.

Anti-goals: a second place that lists the types; a capability the code cannot ask without naming a type; a change to when
an item counts as done for anyone.

## 2. Where it lives

A new crate `backend/crates/learning/activity`. It owns no schema and depends on no layer of ours, so it sits beside the
chain, at the bottom of the learning crates: `content`, `assessment`, `gradebook` and `learning` may use it, `teaching`
does not need to. `server/tests/architecture.rs` lists it as a crate without a schema, and its manifest may name only
`api-types`, `vumlo-config`, `serde_json`, `sqlx`, `thiserror` and `uuid`. `CLAUDE.md` section 4 gets a line for it.

It holds:

- The contract: `ActivityType`, `Capabilities`, `CompletionPolicy`, `Progress`, `ConfigError`.
- The shared data the contract's signature names: `Stage`, `ValidContent`, `AutoGrade`, `Claims`.
- The rich-text document model (`document/`: `EffectiveProfiles`, `DocumentErrors`, `ValidDocument`, the editor
  profiles). Moved from `content` unchanged. The contract's signature names `EffectiveProfiles` and `DocumentErrors`, and
  a type that comes from elsewhere must be able to write the same documents. `content` does not re-export them: the
  eleven callers outside `content` (`flows`, `server`, `ai_agent`) import from `activity` (no compatibility layer,
  CLAUDE.md section 13).
- The registry and `Kind`.

`content` keeps the five built-in types (`types/`), their shared helpers, and `builtin_types()`.

## 3. The contract

```rust
pub trait ActivityType: Send + Sync {
    fn id(&self) -> &'static str;                       // the stored text: "quiz"
    fn capabilities(&self) -> Capabilities;
    fn completion(&self) -> CompletionPolicy;
    // the nine methods of today's AssignmentType, with ConfigError instead of content::ModelError:
    fn validate_config(..) -> Result<(), ConfigError>;  fn points_total(..);  fn strip_computed(..);
    fn validate_submission(..);  fn student_config(..);  fn structure(..);  fn auto_grade(..);  fn claims(..);
    // moved from inherent methods and one `==` check, default "none":
    fn config_file_refs(..) -> Vec<Uuid>;  fn submission_file_refs(..) -> Vec<Uuid>;
    fn check_file_names(&self, config, names) -> Result<(), DocumentErrors>;
}
pub struct Capabilities { pub submissions: bool, pub auto_graded: bool }
```

`takes_submissions()` becomes `capabilities().submissions`. `auto_grade` returning `None` for a type that is not
`auto_graded` stays valid; the capability is what SQL and the work lists ask, the method is what the submit calls.

Adding a capability: a field in `Capabilities` (no default, so every type is made to answer), then the readers. A
capability is added only for a question that code asks about many types without naming one.

## 4. The registry

Same shape as `events::install` (`backend/crates/events/src/dispatch.rs`):

- `static REGISTRY: OnceLock<Types>`; `Types::new().with(&'static dyn ActivityType)` builds it; ids must be unique and
  match `^[a-z][a-z0-9_]{0,31}$`.
- `install(Types) -> Result<(), InstallError>` (twice is an error), called by `server/src/app.rs` and
  `vumlo-cli/src/main.rs` next to `events::install`, with `content::builtin_types()`.
- `get(id) -> Result<Kind, UnknownType>`; `available() -> Vec<&'static str>` (sorted);
  `ids_where(fn(Capabilities) -> bool) -> Vec<&'static str>`.
- Before `install`, `get` is an error (`NotInstalled`), never a silent default. `ensure_installed` (feature `testing`)
  serves tests.

`Kind` is `Copy`: a `&'static dyn ActivityType` with `PartialEq` and `Debug` by id and `Deref` to the trait. It is
`sqlx::Type` and `Decode` as `text` through the registry, so a row with an unknown type is a decode error, never a
guess. `content.assignments.type` stays free text without a CHECK (0006): a new type needs no migration.

## 5. What replaces the type names

| Today | After |
|---|---|
| `BuiltinType::parse` in `routes.rs`, and the list in `ModelError::AssignmentType` | `activity::get`; the message lists `activity::available()` |
| `takes_submissions()` | `capabilities().submissions` |
| `a.type <> 'no_submission'` (gradebook `work.rs`, two queries) | `a.type <> ALL($n)`, `$n = ids_where(\|c\| !c.submissions)` |
| `type = 'quiz'` (gradebook `work.rs`, `repo.rs`) | `= ANY($n)`, `$n = ids_where(\|c\| c.auto_graded)` |
| `== BuiltinType::File` file-name check in `assessment` | `kind.check_file_names(..)` |
| `== Quiz` / `== Form` for the quiz result, form responses, `QuizAttemptSubmitted`, plugin reads | `== quiz::ID` and `form::ID`, constants of the type that owns the feature. These endpoints are the type's own features, not a capability. |
| `== BuiltinType::Text` in `ai_agent` | `== text::ID` |
| `config_file_refs`, `submission_file_refs` (inherent `match`) | trait methods |
| `editor_features` (text type logic inside `model.rs`) | `types/text.rs` |

The CHECKs in migrations 0017 and 0026 name `no_submission`. They stay: they are true for every type that exists. The
migration that adds the first other type without submissions drops them (and says so in its comment, as 0017 already
asks).

## 6. Completion

`CompletionPolicy` is `SubmittedOrGraded` or `Graded`. `Progress { submitted, graded, marked }` is what `content.facts`
holds for a student and an item (`submitted_at`, `published_score`, `marked_at`). `CompletionPolicy::is_met(Progress)`:

- `SubmittedOrGraded`: `submitted || graded`;
- `Graded`: `graded`.

The built-ins: `no_submission` is `Graded`; `text`, `quiz`, `form`, `file` are `SubmittedOrGraded`. `marked` is the
student's own mark and counts for every item that is not an assignment (an assignment refuses `mark_done`, ADR-0092).

`content::repo::facts` reads the three columns and the assignment's type and decides per item: an assignment is done when
`kind.completion().is_met(progress)`, any other item when it is marked. For every row that exists today this gives the
same answer as "a row exists", because the table's CHECK requires one of the three columns, and a `no_submission`
assignment has no submission. `rules.rs` is unchanged: it reads `Facts.done_items`.

The policy is only declared by the type here; the teacher does not choose one. Item 4 adds the per-activity choice and the
events.

## 7. Failure modes

- A DB row with a type no one registered (a module removed): the read fails with a decode error naming the id, not a
  panic and not a wrong guess. The item is unreadable until the module is back; documented in the ADR.
- `install` forgotten in a binary: the first read fails with `NotInstalled`. The server and `vumlo-cli` install it in the
  same function that installs the subscribers, and the architecture test greps that both call it.
- Two types with one id: `Types::with` refuses, at startup.

## 8. Tests

- `activity`: registry (install twice, unknown id, not installed, duplicate id, bad id), `ids_where`, `is_met` table.
- `content`: a table test over the five built-ins for `id`, `capabilities`, `completion` (so a change is a visible
  edit); the existing per-type tests keep running unchanged against the moved code.
- `server` (database): `facts` tests unchanged and green, plus one that a `no_submission` item with a submission-less
  row set is not done until graded; gradebook work lists unchanged.
- `architecture.rs`: the crate order and manifests include `activity`; no `BuiltinType` name is left in the workspace.

## 9. Decisions made

- Assignment types only, not item kinds (the user, 2026-11-14).
- Completion declared and used, with the same result (the user).
- Install-time registry in a small crate (the user).
- The document model moves into `activity`, because the contract names it: one crate instead of two.
- Capabilities are two booleans. `claims`, `structure` and the file hooks are methods, because they have a body per type.
