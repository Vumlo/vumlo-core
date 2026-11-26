# Stage 4 plan: lecture and shared file are activity types

Spec: `docs/superpowers/specs/2026-11-26-activity-generalization-design.md` (ADR-0098), stage 4 of 5, sections 4, 6 and 7.
Built on branch `refactor/activity-resources`, stacked on `refactor/activity-hooks` (stage 3).

## Context

After stage 3 the rules of every graded type are in `activity`, but a lecture and a shared file are still the item kinds
`text` and `file`: a closed enum matched in about fifteen places of `content`, with their body rules in `content/service.rs`.
Stage 4 makes them types (`lecture`, `shared_file`), puts `type` on every item, and moves the body rules behind the contract.
After it, a new kind of content (`link`, `lesson`) is one file in `activity/src/types/` and a registration.

## Departure from the spec (API)

The spec replaced `Item.kind` by `Item.type` and removed `Assignment.type` in the same change as the schema. Counting the
edits (about 210 test sites, every web switch, the mocks) showed that doing both at once buys nothing and hides mistakes.
This stage keeps the API smaller:

- `Item.kind` and `ItemSummary.kind` stay and mean what the database column now means: `resource` (no grade) or
  `assignment` (has points). The values `text` and `file` are gone.
- `Item.type` and `ItemSummary.type` are new, on every item: `lecture`, `shared_file`, or the id of the assignment type.
- The create request gains `type` for a resource (default `lecture`; with an `assignment` object it must be absent, and the
  type is `assignment.type`). `kind` leaves the request.
- `assignment.type` stays for now (it equals `Item.type`). Removing that duplicate is a small follow-up of its own.

## Steps (each commit builds and passes its tests)

### Commit 1: the contract and the two types (no wiring, no visible change)

- `activity::ActivityType` gets the body hooks, with the defaults of an instructions document: `default_body`,
  `validate_body(&Value, profiles) -> Result<Map, DocumentErrors>` (it returns the canonical body, as `document()` and
  `file_body` do today), `body_file_refs`, `check_publishable` (default `Ok`), `reading_minutes` (default `None`).
- `CompletionPolicy::Marked` (met when the student marked it); the comment of `Progress.marked` is reworded.
- `types/lecture.rs` (`lecture`: default hooks, `reading_minutes` from today's `read_minutes`, `Marked`, no tier) and
  `types/shared_file.rs` (`shared_file`: body `{file_id?, description?}` with today's `file_body` rules, refs of `file_id`
  and the description, `check_publishable` needs a file, `Marked`, no tier). `builtin_types()` registers seven types.
- Tests: the table of built-ins gets the two rows and the hooks table; the body rules of `shared_file` and the reading
  time of `lecture` are tested against the functions that already existed in `content` (moved with their tests).

### Commit 2: `type` on every item (schema, content, API)

1. Migration `0058`: `content.items.type` (backfill: `text` to `lecture`, `file` to `shared_file`, an assignment takes
   `assignments.type`), `kind` values `text`/`file` to `resource` with `CHECK (kind IN ('resource','assignment'))`, the views
   `content.item_list` and `content.assignment_list` recreated (`type` from `items`), the CHECKs that name `no_submission`
   and the column `assignments.type` dropped.
2. `content`: `ItemKind` is `{Resource, Assignment}`; `Item` and `ItemSummary` carry `item_type: activity::Kind`; the
   queries that read `a.type` read `i.type` (a `LEFT JOIN` must not decode a lecture as a graded kind); `insert_item` writes
   `type`, `insert_assignment` no longer does; `copy_course` copies `i.type`; `template_counts` counts `resource` as lectures.
3. `content/service`: `validated_body`, `item_file_refs`, `sync_published_files` (the missing-file refusal),
   `begin_create_item` (default body), `read_minutes` and `mark_done` ask the hooks and the completion policy of the item's
   type, not the kind; the facts query asks `kind.completion()` for every row (no more "assignment or not").
4. API: `ItemKind` is `resource|assignment`; `Item.type` and `ItemSummary.type`; `CreateItemRequest.type`; the route
   `new_item` branches on `type.graded()`. `vumlo-cli seed` and the `ai_agent` tools follow.
5. Tests: the requests that create a lecture or a shared file name the type; responses that assert `kind: text|file` assert
   `resource` and the type; the two CHECK tests become API tests of the service rule; `.sqlx` is regenerated once (green
   build only), and `web/openapi.json` with `schema.d.ts`.

### Commit 3: the web app follows

`ItemView` and `ItemEditor` choose the page by `item.type` (`lecture`, `shared_file`, then the assignment type), `kindOf`
and `LecturePage` read the type, the create dialog sends `type`, the mock world stores and returns `type` and the new
`kind`, the e2e specs that name `kind: text|file` follow. `web-admin` and the desktop Rust side need no change.

### Commit 4: docs

ARCHITECTURE.md, CHANGELOG (upgrade note: migration `0058`, the API change), the ADR-0098 line, the spec (the departure).

## Verification

From `backend/`, `CARGO_BUILD_JOBS=2`, one command at a time. Baseline: stage 3 (154 tests in `activity` plus `content`, the
DB suites green except the one known `ai_agent` failure).

1. After each commit: `cargo test --no-run` for every crate, then `SQLX_OFFLINE=true cargo check --workspace --all-targets
   --all-features` (after the regeneration).
2. Commit 1: the new tests only change the count; `.sqlx` has no diff; nothing else changes.
3. Commit 2: a migration proof on a scratch database with old rows (lecture, file, every assignment type, a grade) before
   and after, checking `type`, `kind`, the views and that the foreign key of `assignments` still holds; every DB suite that
   creates items: `content`, `submissions`, `quiz_and_form`, `access_rules`, `courses`, `facts`, `revisions`, `files`,
   `gradebook`, `home`, `authorization`, `plugin_reads`, `ai_agent`; `cargo test -p learning -p content -p assessment -p
   gradebook -p activity`; `vumlo-cli` tests (seed).
4. Commit 3: `tsc --noEmit` in `web/` and `web-admin/`, the unit tests of `web/`, and the e2e specs that run here.
5. `fmt`, `clippy` (also `--features dummy`), `sqlx prepare --check` with and without `--all-features`.
6. A grep outside `activity`, the modules, the CLI, tests and migrations for `ItemKind::Text`, `ItemKind::File`,
   `'text'`/`'file'` as item kinds finds nothing.

## Risks

- **Decode of a lecture as a graded kind.** Every `LEFT JOIN content.assignments` that reads the type must read it from the
  item and only when the assignment row exists.
- **`FOR SHARE` of the assignment row** (`lock_assignment_shared`) must keep locking `content.assignments` only; the type is
  read from `items` without locking it (`FOR SHARE OF a`).
- **Stored bodies stay canonical:** the body hook returns the body to store, as `document()` and `file_body` do.
- **Size.** If the web part does not fit one sitting, commit 3 is the line to stop at: the backend is complete and green
  without it, but the web app then reads the old kinds and is not usable against it until commit 3 lands.
