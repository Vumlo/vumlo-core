# Split `learning`, PR B2: the reads that go down go through declared views. Implementation record

**Goal:** after B1 nothing goes up; B2 makes every read of a lower module a read of a view that module declared, and empties the `RATCHET_DOWN` list in `architecture.rs`.

**Spec:** `docs/superpowers/specs/2026-11-12-split-learning-design.md` section 4. ADR-0093.

## Decisions

- One migration, `0054_read_views_between_modules.sql`: eleven plain views, one per source table that another module reads, in the style of `0033_read_views.sql` (a view on one table, so Postgres reads through it and a row lock taken through it locks the table's rows). No table, row or API changes.
- What a view leaves out is the point of it:
  - `content.item_list` has no `body` or `published_body`; its `edited` column says that a published item has a working title or body students do not read (the one reader that compared the texts, the unpublished-content list of the gradebook, asks that).
  - `assessment` has two views of `submissions`: `submission_index` (which drafts and attempts exist, no `content`, no scores) for the readers that count or check ownership, and `submitted_attempts` (submitted rows only, with `content` and `auto_scores`) for the plugin reads, so a draft's content never leaves `assessment`.
  - `gradebook.published_grades` has the `published_*` columns only, and only rows with something published, so a reader above `gradebook` cannot select a draft (CLAUDE.md section 6).
  - `teaching.course_list`, `announcement_list`, `tombstone_list`, `content.module_list`, `assignment_list`, `item_file_list`, `assessment.submission_file_list`: the columns the readers use, nothing hidden.
- A view column is nullable to sqlx. Every query that returns one into a non-optional field writes `AS "col!"`, as the queries over `auth.people` do.
- `architecture.rs`: `DECLARED_VIEWS` gains the eleven; `RATCHET_DOWN` and the test that its entries still happen are deleted, so the read rule has no list. `server/tests/read_views.rs` pins what the views leave out.

## Done

- Migration 0054 and `DECLARED_VIEWS`.
- Readers switched: `assessment/repo.rs`, `audit/repo.rs`, `content/repo.rs` (one join), `facts/repo.rs`, `files/repo.rs`, `gradebook/repo.rs`, `gradebook/repo/{groups,work}.rs`, `plugin_read/repo.rs`.
- Tests: `read_views.rs` (3). The behaviour tests that cover every reader (gradebook, submissions, quiz_and_form, plugin_reads, plugin_events, files, home, content, courses, teaching, groups_roster, admin_reports, schedule, access_rules, revisions, facts, authorization) pass unchanged.
- Docs: ARCHITECTURE.md rule 2, CLAUDE.md section 4, CHANGELOG (upgrade note), ADR-0093 line, PLAN.md.

## Next

PR C: `git mv` into `backend/crates/learning/{teaching,content,assessment,gradebook,flows}`, five crates with their own errors, state and routers; CI paths, `Dockerfile`, `RELEASING.md`, docs.
