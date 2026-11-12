# Splitting `learning` into crates: design

Status: draft for review. Path: architectural. ADR-0093. Delivery: one spec, three stacked PRs (A, B, C).

## 1. Goal

`learning` holds 23.5k of the 29.4k non-test lines of the backend, in four modules (`teaching`, `content`,
`assessment`, `gradebook`) that read each other's tables freely and call each other in a circle. A new module
(question bank, progress, forums) has nowhere clean to go, and Cargo cannot stop a wrong dependency.

Deliver:

1. Five crates under `backend/crates/learning/`: `teaching`, `content`, `assessment`, `gradebook`, and `flows`
   (the crate `learning`). Cargo enforces the order `teaching < content < assessment < gradebook < learning`.
2. Reads across crates only through declared views (the ADR-0065 rule, already used for `auth` and `org`).
3. Every call that goes "up" that order becomes a flow in `learning`, or a subscriber of an event.
4. Access rules no longer read `assessment` and `gradebook`. `content` keeps a projection of the facts it needs
   (`content.facts`), updated in the same transaction by synchronous event subscribers, and rebuildable from the
   sources of truth.

Out of scope (own specs): org units (ADR-0094), the activity type contract replacing `BuiltinType`, the progress
module and `activity.completed`, new modules, interop. **No API, no behaviour and no screen changes**: OpenAPI is
byte-identical after each PR, and no existing test changes its assertions.

## 2. Decisions

| Topic | Decision |
|---|---|
| Layout | `backend/crates/learning/{teaching,content,assessment,gradebook,flows}`; `flows` is the crate named `learning`. |
| Order | `teaching < content < assessment < gradebook < learning < plugins < server`. |
| Reads | Across crates only through views the owner declared (`DECLARED_VIEWS`). Views of grades expose published values only. |
| Upward calls | A flow in `learning` (one function, one transaction), or a synchronous subscriber installed by the composition root. |
| Facts | `content.facts` is a projection owned by `content`; `assessment` and `gradebook` stay owners of their data. |
| Sync events | `events::emit` writes the outbox row and runs the installed subscribers on the same connection. Any error aborts the operation. |
| Registry | Built by the composition root, installed once with `events::install`, immutable after (`OnceLock`). `events` depends on no higher crate. `emit` with no registry installed is an error. |
| Async | Plugins, mail and other consumers stay on the transactional outbox (ADR-0007, ADR-0045). |
| Completion | `activity.completed` is not emitted here. When it exists, an activity's completion policy decides; `submission.submitted` alone never completes anything. |

## 3. Crates and layers

| Crate | Schema | Depends on (new) | Public surface |
|---|---|---|---|
| `teaching` | `teaching.*` | `org`, `auth`, `files`, `events`, `access`, `common`, `api-types` | courses, roster, group links, exclusions, announcements, tombstones, `effective_members` |
| `content` | `content.*` | `teaching` + the above | modules, items, assignments, rules, `facts`, `standing`, copy, search |
| `assessment` | `assessment.*` | `content`, `teaching` + the above | drafts, submissions, quiz scoring, form answers, lateness |
| `gradebook` | `gradebook.*` | `assessment`, `content`, `teaching` + the above | grades, publication, history, targets, groups, export |
| `learning` | none | all four | the flows: people, groups, reports, plugin reads, scheduler, file access, audit report, course lifecycle, item read models |

- Each crate has its own error enum, state (only what it needs: `content` owns the editor profiles, `files` handles
  go to the crates that serve files) and router with its OpenAPI paths; `server` merges them as it does `auth` and
  `org`. `points.rs` moves into `content` (used by `content`, `gradebook`, `plugin_read`).
- `architecture.rs` gets one `Unit` per crate with its own layer number (7 to 11; `plugins` and `server` move up).
  Its source scan also flags `crate::<higher module>::` imports during PR B, before Cargo does the same in PR C.
  PR B is delivered in two steps: B1 removes every read and call that goes *up* the order (a ratchet list that
  ended empty, then deleted), B2 declares the views for the reads that go *down* (`RATCHET_DOWN` shrinks to empty).
- The `learning` error enum wraps the others, as it does for `teaching` today. The error variants `teaching` has for
  `content`, `assessment` and `gradebook` disappear with the upward calls.

## 4. Reads

Measured on `main` (non-test code): about 150 schema-qualified references cross the four modules. Per crate:

| Reader | Reads of | Tables |
|---|---|---|
| `content` | `teaching` | `courses`, `effective_members` |
| `assessment` | `teaching`, `content` | the same, plus `modules`, `items`, `assignments`, `blocks`, `files` |
| `gradebook` | `teaching`, `content`, `assessment` | `effective_members`, `items`, `modules`, `assignments`, `submissions` |
| `learning` (flows, `plugin_read`, `files`, `audit`) | all four | items, modules, assignments, item and submission files, submissions, announcements, tombstones, grades |

Each becomes a view named for its purpose, declared in migration `0054` and listed in `DECLARED_VIEWS` (eleven were
needed). The set was drawn up from the ratchet in `architecture.rs`, which listed every cross-schema reference. Rules:

- A grades view exposes `published_*` columns only. A reader above `gradebook` cannot select a draft (CLAUDE.md §6).
  Staff reads of drafts go through `gradebook` functions.
- A view carries what readers need, not whole tables; a reader that needs a column adds it in a migration.
- `teaching.effective_members` is declared too; the roster stays read through it only (ADR-0033).
- `teaching` no longer reads any upper schema.

## 5. Upward calls

| Today | Becomes |
|---|---|
| `teaching`: delete a course (guards and purge in `gradebook`, `assessment`, `content`) | flow `learning::courses::delete`: asks the guards, purges the upper modules top-down, then `teaching::delete_course` |
| `teaching`: publish a course (`content::announce_visible`) | flow `learning::courses::publish`: `teaching::publish`, then `content::announce_visible`, one transaction |
| `teaching`: course header, people, candidates, cards (join `content`, `assessment`, `gradebook`) | read models in `learning::courses` over declared views |
| `content`: delete a module, an item or an assignment (guards, purge) | flows `learning::items::delete`, `learning::modules::delete`: guards first, purge top-down, then the `content` delete |
| `content`: assignment edit guards (submissions exist, highest score) | `content::update_assignment` takes an `EditGuard` argument; the flow fills it from `assessment` and `gradebook` |
| `content`: form facts in the opened assignment (`assessment::form_facts`) | `GET /items/{id}` and the opened assignment become a flow that adds the taken options |
| `assessment`: quiz auto-grade (`gradebook::save_auto_grade`) | subscriber of `submission.submitted`, registered before the facts one so that a submit takes the grade row before the facts of the pair, in the order publishing does |
| `assessment`: the student's own result (`gradebook::published_grade`) | flow `learning::results::get` |
| `content` facts for rules (`assessment`, `gradebook` in `repo::facts`) | `content.facts` projection (section 7) |

Routes follow their functions: a flow keeps the path, method and DTOs it has today.

## 6. In-transaction event dispatch (`events`)

```rust
pub struct Subscribers { /* by event kind, in registration order */ }
impl Subscribers { pub fn new() -> Self; pub fn on(self, kind: &'static str, handler: SyncHandler) -> Self; }
pub fn install(subscribers: Subscribers) -> Result<(), EventsError>;   // once; a second call is an error
pub async fn emit(conn: &mut PgConnection, event: Event) -> Result<(), EventsError>;
```

- `SyncHandler` takes the connection and the event, returns `Result<(), SubscriberError>`. Handlers run sequentially in
  registration order, inside the caller's transaction, after the outbox row is written. An error aborts the
  operation (the caller returns it, the route rolls back).
- `emit` reads the installed registry (`OnceLock`); with none installed it returns `EventsError::NoSubscribers`
  instead of skipping silently. `emit_with(conn, event, &Subscribers)` is the same without the global, so the tests of
  `events` build their own registry.
- The composition root builds the registry: `learning::subscribers()` returns the subscribers of the flows; `server`
  (both `serve` and `worker`), the test harness of crates that emit, and any CLI command that emits call
  `events::install`. `events` never names `learning`.
- Handlers must be idempotent: a repeat gives the same state. Payloads stay ids only (CLAUDE.md §7); a handler
  reads what changed through the owner's public function.
- New internal event `GradePublicationChanged { assignment_id, student_id, course_id }` (kind `grade.publication_changed`,
  not delivered to plugins, like `group.members_changed`). `gradebook::publish_grades` emits it for every published
  row, including a retraction to an empty value, which emits nothing today (`grade.published` is unchanged: only when
  a value became non-empty).

## 7. The facts projection (`content.facts`)

```
content.facts(item_id uuid FK content.items ON DELETE CASCADE, student_id uuid, submitted_at timestamptz NULL,
              published_score numeric NULL, marked_at timestamptz NULL, PRIMARY KEY (item_id, student_id),
              CHECK (submitted_at IS NOT NULL OR published_score IS NOT NULL OR marked_at IS NOT NULL))
```

- An item is done when its row exists (an assignment: submitted or scored; any other item: marked).
  `standing` reads this table only; `content::repo::facts` stops touching `assessment` and `gradebook`.
- Writers (all idempotent upserts, a row with all three empty is deleted):
  - `content::mark_done` / `unmark_done` set and clear `marked_at` (today's `content.completions`).
  - subscriber of `submission.submitted`: sets `submitted_at` to the first submitted attempt of the student (read
    through `assessment`). A submitted attempt is never deleted (only drafts are), so it never clears.
  - subscriber of `grade.publication_changed`: sets `published_score` from `gradebook`, or clears it.
- One function in `learning` reads the sources for `(item, student)`; the subscribers and the rebuild both call it,
  so they cannot disagree.
- `learning::rebuild_facts(conn, course?)` recomputes the rows from the sources; `vumlo-cli facts rebuild [--course]`
  runs it and writes an audit row (CLAUDE.md §4). Migration `0051` creates the table and backfills it from the
  current data, `0052` drops `content.completions`, and `0053` adds `assessment.submissions.auto_total`, the
  sum the quiz subscriber turns into the draft grade.
- Student removed from a course or item deleted: as today, the rows stay for the first and cascade for the second.

## 8. Delivery

Each PR keeps the API and every existing test green, is reviewed by the user, then merged before the next.

| PR | Content | Proof |
|---|---|---|
| A. Dispatch and facts | `events::install` and sync subscribers; migrations `0051` to `0053`; `GradePublicationChanged`; subscribers (facts, quiz auto-grade); `rebuild_facts` and its CLI command; `content` stops reading `assessment` and `gradebook` | `events` unit tests (no registry, second install, handler order, handler error); a scenario test that the projection equals `rebuild_facts` after submit, publish, re-publish, retract, mark, unmark; a failing subscriber rolls the write back; `access_rules.rs` and `submissions.rs` unchanged |
| B. Boundaries in one crate | flows replace the upward calls; migration `0054` declares the views; `architecture.rs` splits the layers (`UNITS`, `DECLARED_VIEWS`, import scan) | `architecture.rs` green with the new layers; the ratchet list is empty; OpenAPI identical |
| C. The move | five crates, workspace members, per-crate error, state and router, `server` composition; CI paths, `Dockerfile`, `RELEASING.md`, `CLAUDE.md` section 4, `ARCHITECTURE.md`, `docs/PLAN.md`; `git mv` keeps history | `cargo check` per crate; Cargo refuses a wrong dependency; OpenAPI identical; all tests green |

Local checks are targeted (`CARGO_BUILD_JOBS=2`, one command at a time): fmt, clippy `-D warnings`, `architecture.rs`,
`authorization.rs`, `access_rules.rs`, `schedule.rs`, `submissions.rs`, `gradebook.rs`, `quiz_and_form.rs`,
`plugin_events.rs`, `cargo sqlx prepare --check`. The full workspace and `--features dummy` runs are left to CI.

## 9. Risks

- **A form item's student view becomes a flow** (section 5): its route moves from `content` to `learning`. It is the
  one flow whose route is on the read path of every item; a test per item kind guards it.
- **The global registry in tests.** Every test process that emits installs the registry once (a `Once` helper per
  crate with a `testing` feature, as `Actor::for_tests` does). A missing install fails loudly, which is the point.
- **PR B is wide.** About 150 references change shape. The ratchet in `architecture.rs` lists them first, and the
  work goes by reader crate, so each commit is green. B1 (done) moved the read models that sat too low
  (`/me/work`, the item facts and the figures of the cards into `gradebook`; the draft and deadline queries into
  `content`) and made flows of everything that called up (`learning::courses`, `learning::items`); B2 declares the
  views.
- **PR C is mostly moves.** `git mv` first, edit after; `.sqlx/` is regenerated and checked.
- **Ordering of subscribers** is registration order. If two subscribers of one event ever depend on each other, that
  is a sign they are one subscriber.
