# Activities: one concept, one crate

Status: accepted by the owner on 2026-11-26 (ADR-0098), not built. Roadmap item 3 follow-up, and the precondition of
roadmap items 4 and 5. No installation is running and no plugin is compatible with this system, so nothing here carries a
compatibility or rollout obligation; where a break is the cleaner design, it is made.
Builds on ADR-0097 (`docs/superpowers/specs/2026-11-14-activity-types-design.md`), which made the *assignment type*
a contract and a registry. This spec finishes the job: every piece of course content is an activity of some type, and
everything a type decides lives in one crate.

## 1. Intent

ADR-0097 left two kinds of thing outside the contract:

1. **Lectures and shared files.** An item has a kind (`text`, `assignment`, `file`), a closed enum with a DB CHECK
   (`content/src/model.rs:73`, migration 0038:4), matched in about ten places of `content/src/service.rs`. Only
   assignments have a type. A new kind of content (`link`, `lesson`, ADR-0024 and ADR-0026) means editing all of them.
2. **The features of a type.** The quiz result, the form responses, the "taken" overlay of a form, the quiz event and
   the plugin reads are served by `assessment`, `content`, `flows` and `plugins`, which compare the type id with
   `quiz::ID` and `form::ID` (`assessment/src/service.rs:581,747,796`, `content/src/service.rs:1309`,
   `flows/src/plugin_read/service.rs:94,220,331,355`). The five types themselves sit in `content/src/types/`
   (about 2.1k lines) although they import nothing of `content` but `ModelError` and `MaxPoints`.

Deliver:

1. **One concept.** A course module holds *activities*. A lecture, a shared file, a text assignment, a quiz and a form
   are all activities, each of a registered type.
2. **One crate.** `activity` holds the contract, the registry, the document model and every built-in type. The rules of
   a quiz, the formatting rules of a lecture and the body of a shared file are written there and nowhere else.
3. **No type names elsewhere.** The crates of the learning chain, `events`, `access` and `plugins` ask the contract or
   a capability. They never compare with a type id.
4. **A way in for modules.** A module (a crate behind a Cargo feature, ADR-0066) registers a type of its own, with no
   edit to the core. This is what forums, a question bank or `link` and `lesson` need.

Non-goals:

- A per-activity completion policy chosen by the teacher, and the events `activity.completed` and `course.completed`
  (roadmap item 4). This spec only moves the policy that exists today behind the contract.
- The open catalogues of events, plugin scopes and `access::Action` for modules (review finding B2). Section 7 removes
  the quiz names from them; making them extensible is a separate decision.
- Web pages for module-provided types. The per-type pages of the built-ins stay (section 8).
- Any change to grading, deadlines, access rules, revisions or visibility.

Anti-goals: a second place that lists the types; a capability that cannot be asked without naming a type; a stub method
on a type that cannot use it; a compatibility layer (CLAUDE.md section 13).

Terms. *Activity* is the concept, and the crate is `activity`; the tables and the API keep the word *item*
(`content.items`, `/items/{id}`), because renaming them buys nothing. A *graded* activity is today's assignment: it has
points, a config and a gradebook column.

## 2. Where it lives

```
activity/src/
    contract.rs, model.rs   the traits and the data their signatures name        (today)
    registry.rs             install, get, ids_where, Kind                         (today)
    document/               the rich-text model                                   (today)
    points.rs               MaxPoints, PointsError, parse_decimal: the rules of a point value   (from content)
    types/                  lecture, shared_file, text, no_submission, file, form, quiz   (from content, two new)
```

`content` keeps the structure: modules, the item rows and their revisions, access conditions, `content.facts`,
scheduling. `builtin_types()` and the type files leave it. `activity` stays a crate with no schema, beside the chain, at
layer 7 (`server/tests/architecture.rs:86`). Its manifest rule (ADR-0097) is unchanged; the types add `chrono`, which
the test already accepts. The types return `ConfigError` instead of `ModelError`, and `MaxPoints`
(`types/quiz.rs:153`) moves into `activity` under its own name, with a new `PointsError` of the same text; `content`
uses it for the points of an assignment and maps the error to `ModelError::MaxPoints`. (A rename to `Points` was
dropped: it touches about 25 call sites and rewrites 8 `.sqlx` queries, and a move needs neither.)

A type does no I/O. Anything it needs from the database (the files a document names, the claims of other students) is
passed in, as `check_file_names` already does (`types/file.rs:127`).

## 3. The contract in tiers

Today one trait of eleven required methods serves every type, so `no_submission` writes seven of them as stubs
(`points_total`, `strip_computed`, `validate_submission`, `student_config`, `structure`, `auto_grade`, `claims`:
`content/src/types/no_submission.rs:51-92`). Adding a lecture and a shared file would add two more types that are
almost all stubs. The contract becomes three tiers; a type implements the tiers it has.

```rust
pub trait ActivityType: Send + Sync {                  // every activity
    fn id(&self) -> &'static str;
    fn completion(&self) -> CompletionPolicy;          // Marked | SubmittedOrGraded | Graded
    fn graded(&self) -> Option<&dyn Graded> { None }
    // the item's own body (title is shared):
    fn default_body(&self) -> Value;                   // default: the empty document
    fn validate_body(&self, body: &Value, profiles: &EffectiveProfiles) -> Result<(), DocumentErrors>;
                                                       // default: a Full-profile document (lecture, instructions)
    fn body_file_refs(&self, body: &Value) -> Result<Vec<FileRef>, DocumentErrors>;  // default: refs under "body"
    fn check_publishable(&self, body: &Value) -> Result<(), ConfigError>;            // default: Ok (shared_file needs a file)
    fn reading_minutes(&self, body: &Value) -> Option<u32> { None }                  // lecture
}

pub trait Graded: Send + Sync {                        // points, config, a gradebook column
    fn validate_config(..); fn points_total(..); fn strip_computed(..);
    fn structure(..);       fn config_file_refs(..);
    fn student_config(&self, config: &Map, held: &Held) -> Map;   // `held`: see section 5
    fn plugin_config(&self, config: &Map) -> Option<Value> { None }
    fn submissions(&self) -> Option<&dyn Submissions> { None }
}

pub trait Submissions: Send + Sync {                   // a student hands something in
    fn validate_submission(..); fn claims(..);
    fn submission_file_refs(..); fn check_file_names(..);
    fn auto_grading(&self) -> Option<&dyn AutoGrading> { None }
    fn review(..)  -> Option<Review>    { None }       // what a student sees after the grade is published
    fn responses(..) -> Option<Responses> { None }     // what staff see of everyone's answers
    fn plugin_answers(..) -> Option<Value> { None }
}
pub trait AutoGrading: Send + Sync { fn auto_grade(..) -> AutoGrade; }
```

`Capabilities` stays plain data for SQL: `{ graded, submissions, auto_graded }`, now **derived** from which tiers a type
returns, so a type cannot claim what it does not implement. `ids_where` binds it as before (`gradebook/service/work.rs:65,77,87`).
`Kind` derefs to `ActivityType`; a call site reads `kind.graded()` and gets an `Option`, so asking a lecture for its
points is a type error, not a runtime refusal. `validate_submission` and `auto_grade` are no longer "called only for a
type whose capabilities take submissions" by convention; they exist only on the tier that has them.

`CompletionPolicy` gains `Marked` (the student marks it done by hand). Lecture and shared file are `Marked`; the three
policies of the graded types are unchanged. `content/src/repo.rs:1516-1555` stops special-casing "assignment or not":
every item is done by `kind.completion().is_met(progress)`, and `mark_done` is allowed when the policy is `Marked`
(`content/src/service.rs:2236-2262` refuses assignments today). The result for every existing row is the same.

**As built in stage 2** (`docs/superpowers/plans/2026-11-26-activity-stage-2-tiers.md`). Three departures from the
sketch above. (1) Four hooks of `Graded` have a neutral default (`points_total`, `strip_computed`, `student_config`,
`structure`), as `config_file_refs` and the two file hooks of `Submissions` already had, so a type writes only what it
has; `validate_config`, `submissions`, `validate_submission`, `claims` and `auto_grading` stay required. (2) The type of
an assignment is `activity::GradedKind`, a `Kind` known to be graded, instead of an `Option` read at every call site: the
rows of `content.assignments` hold graded types only, the decode refuses any other, and `Deref` gives the graded tier.
`Kind::graded()` and `get_graded` are the way in. (3) `auto_grade` keeps its `Option<AutoGrade>`: a quiz whose config
cannot be read is left to a teacher. `CompletionPolicy::Marked` and the body hooks of `ActivityType` come with the two new
types in stage 4.

## 4. Lecture and shared file as types

| Type | Replaces | Body | Tiers | Completion |
|---|---|---|---|---|
| `lecture` | item kind `text` | a Full-profile document (`service.rs:892-904`) | none | `Marked` |
| `shared_file` | item kind `file` | `{file_id?, description?}` (`service.rs:848-886`); publishing without `file_id` is refused (`FileMissing`, `service.rs:1039`) | none | `Marked` |

Their rules move out of `content/src/service.rs` into the type: the body validator, the default body (`service.rs:1139-1142`),
the file references of the body (`item_file_refs`, `service.rs:915-960`), the reading time of a lecture
(`lecture_bodies`, `repo.rs:1148-1163`, feeding `read_minutes`) and the publish check. `content` calls the hook and
stays ignorant of what a body contains. The ids `lecture` and `shared_file` are new on purpose: the assignment types
`text` and `file` already exist, and renaming stored ids would break plugins (`assignments:read` exposes the type).

A future `link` or `lesson` is one more file in `types/` (or in a module): an id, a body validator, a completion policy.

## 5. Features move behind hooks

| Today | After |
|---|---|
| `begin_quiz_result` is gated by `id() == quiz::ID`, calls `content::quiz_result_questions` (`assessment/service.rs:740-775`) | `Submissions::review(config, content, scores) -> Option<Review>`; `Review` is a list of questions with the answer, the key, the explanation and the points. The route `GET /assignments/{id}/result` returns it or `404` |
| `responses` is gated by `form::ID` (`assessment/service.rs:796`) | `Submissions::responses(config, rows) -> Option<Responses>` (a table: field, answers per student) |
| `content::decorate_form`, `FormView`, `Holders` (`content/service.rs:1244-1266,1309`) | `Graded::student_config(config, held)`, where `Held` is what `assessment` computes from `claims()` for any type: which exclusive keys are taken, never by whom (ADR-0051). Only the form reads it |
| the plugin reads of a form's fields and answers (`plugin_read/service.rs:94,220`) | `Graded::plugin_config` and `Submissions::plugin_answers`; `assignments:read` carries the config a type chooses to expose |
| `Event::QuizAttemptSubmitted` emitted when `id() == quiz::ID` (`assessment/service.rs:581-591`) | emitted when `capabilities().auto_graded`; renamed, see section 7 |
| `Scope::QuizzesRead` and the routes `/plugin-api/assignments/{id}/quiz` and `/attempts` (`plugin_read/service.rs:326-380`) | `Scope::AttemptsRead`; `/attempts` for any auto-graded type; the quiz questions come through `plugin_config` |
| `ai_agent` writes text answers (`tools/mod.rs:511`, `tools/submitting.rs:123`) and quiz configs (`tools/authoring.rs:339,364`) | a module may name a built-in id, through `activity::types::<name>::ID`; it is the module's own feature. Rule in section 9 |

`exclusive` and `people` stay in `Claims`; the advisory lock named `form:<id>` (`assessment/repo.rs:373-378`) becomes
`claims:<id>`.

**As built in stage 3** (`docs/superpowers/plans/2026-11-26-activity-stage-3-hooks.md`). Reading the code made two of
the data hooks above simpler. The responses are already generic rows, so the hook is the bool `lists_responses`, not a
`responses` method; the overlay is `Graded::annotate_config` over `ClassFacts` (which `assessment` computes for any type
from `claims()`), with `reads_class_facts` telling `content` when to ask; the review is a tier,
`Submissions::reviewing` with `Reviewing::review`. The plugin reads are `Graded::plugin_config` (what `assignments:read`
shows as `config`) and `Submissions::plugin_answers`. The API of the web app is unchanged (`QuizResult` and its route keep
their names; they change with `Item.type` in stage 4). Two things the spec did not foresee: `plugin_installs.api` was
pinned to `'1'` by a CHECK and written as a literal, so migration `0057` relaxes it and the install and the upgrade now
write `plugins::API_VERSION`; and the renamed scope and event are also renamed in what is already stored (the installs, the
deliveries, the outbox).

## 6. Data model

One migration, written with the services stopped (section 10).

1. `content.items` gets `type text NOT NULL`. Backfill: `text` becomes `lecture`, `file` becomes `shared_file`, an
   `assignment` takes the type of its row in `content.assignments`.
2. `content.items.kind` stays, with a structural meaning: `resource` (no grade) or `assignment` (has a row in
   `content.assignments`). Values `text` and `file` become `resource`; the CHECK becomes `kind IN ('resource','assignment')`.
   The foreign key `(item_id, item_kind) -> items(id, kind)` of 0006:19-20, the tombstone trigger (0046:41-52) and the
   index on `kind = 'assignment'` (0049:16) keep working untouched. This is the one closed set left, and it is a real
   one: a thing has a grade or it has not.
3. `content.assignments.type` is dropped; the type of every activity is `items.type`. The views `content.item_list` and
   `content.assignment_list` (0054) are recreated: `item_list` exposes `type`, `assignment_list` reads it from `items`.
4. The two CHECKs that name `no_submission` go: `assignments_no_submission_settings` (0017:18-19) and
   `assignments_class_on_check` (0026:21-22). The service already refuses a deadline or a class date on a type without
   submissions through capabilities (`content/routes.rs:355-360`, `service.rs:1863-1871`). Tradeoff: the database no
   longer rejects it as a second line. A test through the API pins the rule.
5. `max_points > 0` stays: only graded activities have a row there.

`content.assignments` keeps its columns; `class_on` stays a column for now (a settings object per type is later).

## 7. API and protocol changes

These are breaks, allowed before 1.0 and recorded in the CHANGELOG.

- `Item.kind` (`text|assignment|file`) is replaced by `Item.type` (the id, a string) on every item. `Item.assignment`
  stays and means "graded"; `Assignment.type` is removed, since it duplicates `Item.type`. `ItemSummary` and the
  search hit follow. A search hit is `Assignment` when graded and `Lecture` otherwise.
- `TemplateCounts.lectures` counts the non-graded activities, so shared files are counted too (they were not).
- Plugin protocol, in stage 3: scope `quizzes:read` becomes `attempts:read`, the event `quiz.attempt_submitted` becomes
  `attempt.autograded`, and the `/quiz` route disappears (section 5). The plugin `api` goes to `"2"`. The SDK
  (`FakeCore`, the manifest check) and the catalogue in the repository `vumlo-plugins` are not part of this change; no
  plugin runs against this system today, and the SDK is brought to `api = "2"` as its own piece of work.

The OpenAPI file and `web/src/lib/api/schema.d.ts` are regenerated.

## 8. Clients

- `web/`: `ItemView` and `ItemEditor` switch on `item.type` instead of `kind` and `assignment.type`
  (`components/item/ItemView.tsx:27-35`, `components/author/ItemEditor.tsx:29-35`); the lecture and shared-file pages
  keep their components. `isAssignment` and `kindOf` (`lib/course.ts:42-43`) read `item.assignment`. About fifteen files
  branch on kind or type today. The mock world follows the server (`web/src/mocks`, about 3000 lines); this is the
  largest single edit of the change and the reason web gets its own stage.
- `desktop/`: shares the web code; its offline store keys on `kind === "assignment"` (`lib/offline.ts:204`).
- `web-admin/`: no change.
- A module's web page for its own type arrives through the module web surface (ADR-0071), not through this spec.

## 9. Enforcement

`server/tests/architecture.rs`:

- No built-in type id appears as a string literal or a `types::<name>` path outside `activity`, except in modules
  (`crates/modules`), `vumlo-cli`, tests and migrations. A new rule scans the crates it names today as free: `db`,
  `vumlo-cli`, `modules/*` (review finding A2).
- `activity` lists no crate of ours but `api-types` and `vumlo-config` (unchanged).
- No allowlist of leftovers: `events`, `access` and `plugins` name no type once stage 3 is done.

## 10. Migration and rollout

- **Forward only.** There are no down migrations (ADR-0055 item 8). No installation is running, so there is no data to
  carry over and no backup to take; the migration still backfills correctly, because a development database exists.
- **No mixed-version window.** The migration changes the meaning of `kind` and drops `assignments.type`, so an old
  server cannot read the new rows. With nothing running this costs nothing; a future installation updates through
  `update.sh`, which stops the services first.
- **Checkable.** The backfill is one statement per kind. `vumlo-cli doctor` reports activities whose `type` is not
  registered.

## 11. Stages

Each stage is a PR that leaves the system green and changes no behaviour a user can see, except where it says so.
Stages follow the repository's habit: one plan, one PR, a stop for review.

| # | Stage | Visible change |
|---|---|---|
| 1 | **Move.** The five types, `MaxPoints` and the helpers go from `content` to `activity/src/types/`; `ConfigError` replaces `ModelError`; the exports `content::{quiz,form,text}::ID` become `activity::types::..::ID` | none |
| 2 | **Tiers.** The contract becomes `ActivityType` / `Graded` / `Submissions` / `AutoGrading`; the stubs of `no_submission` disappear; `Capabilities` is derived; call sites read `kind.graded()` | none |
| 3 | **Hooks.** `review`, `responses`, `Held` and `student_config`, `plugin_config`, `plugin_answers` replace the type comparisons in `assessment`, `content`, `flows`, `plugins`; the event and scope renames of section 7 | plugin protocol `api = "2"` |
| 4 | **Activities.** `lecture` and `shared_file`, the body hooks, `CompletionPolicy::Marked`, the migration of section 6, `Item.type`, the web and mock changes, regenerated OpenAPI | `Item.kind` becomes `Item.type`; template counts |
| 5 | **Modules.** `Registrar::activity_type`, installed before `activity::install`; an unregistered stored type is handled (D4); `doctor`; a test module type | none for built-ins |

Rough size: stage 1 moves 2.1k lines and edits about 40 imports; stage 2 touches the five types and about fifteen call
sites (the table of `ADR-0097` section 5 lists them); stage 3 about twelve; stage 4 about ten sites in
`content/src/service.rs`, one migration, about fifteen web files; stage 5 about a hundred lines.

## 12. Failure modes

- **A stored type nobody registered** (a module was removed). Today `Kind` decodes as an error
  (`activity/src/registry.rs:81-86`), so one such row would fail the read of a whole course tree. With modules that can
  be switched off this is no longer an edge case. Recommended: the registry hands out a `Kind::Missing(id)` that has no
  tiers and a body that is never shown; the tree lists it as unavailable; `doctor` names it (section 14, D4).
- **Id clash** between a module type and a built-in, or two modules: `Types::with` refuses at startup (ADR-0097).
- **`install` forgotten**: `NotInstalled` on the first read, as now.
- **A module installed after the registry is built**: the composition root registers modules first, then installs
  once.
- **Migration on a database with an unknown `kind`:** the backfill fails on the CHECK and the transaction rolls back.

## 13. Tests

- `activity`: a table test over the seven types for `id`, tiers, capabilities, completion (a change is a visible
  edit); the existing per-type tests move with the code and run unchanged; registry tests with a fake type with tiers
  and one with none.
- Contract tests for each new hook with the data of today's quiz and form tests, so the result is identical.
- `server` (database): the visibility invariants of CLAUDE.md section 6 (no answer key, no draft grade, locked items)
  rerun unchanged; the migration test seeds old rows (`text`, `file`, five assignment types) and checks `type`, `kind`
  and the views; a deadline on a type without submissions is `422`.
- `architecture.rs`: the rules of section 9.
- `web`: `typecheck` and `build`; the mock world and its e2e specs updated, and one e2e run against the real server for
  the item pages (the mock cannot prove the new shape).

## 14. Decisions

Settled with the owner (2026-11-26):

- Lectures and assignments are one concept, handled by one crate that owns the rules of tests and the formatting of
  lectures.
- A module is another feature of the core that is switched on or off; it may use the core's crates. Plugins are the
  untrusted, external kind.
- Before 1.0 the core breaks and is rebuilt wherever that gives the cleaner design. No installation is running and no
  plugin is compatible with this system, so there is no rollout, backup or compatibility obligation.
- Where the owner left the choice to the author, the recommendation below is taken.

Taken:

- **D1. Tiers, not one flat trait** (section 3). A flat trait keeps stubs and methods that must not be called.
- **D2. The plugin-visible names are renamed now**, in stage 3, with `api = "2"` (section 7). With no plugin running the
  cost of waiting would be three names that say `quiz` in `events`, `access` and `plugins`, for no benefit.
- **D3. Ids `lecture` and `shared_file`.** Existing ids keep their spelling: renaming `text` or `file` would gain
  nothing now that the item kinds of the same name are gone.
- **D4. An unregistered stored type is `Kind::Missing`**, not a startup failure, so switching a module off never makes a
  course unreadable.
- **D5. No rollout window.** The migration of stage 4 is one forward migration.
- **D6. Service-only enforcement** replaces the two dropped CHECKs (section 6, point 4); a stored flag would duplicate a
  capability that code can change.
