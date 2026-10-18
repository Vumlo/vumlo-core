# 0049. Unpublished revisions, class dates, per-item facts and grade history

Status: Accepted
Supersedes: ADR-0010 (its last decision: edits to published content are visible at once)
Supplements: ADR-0009, ADR-0021, ADR-0024, ADR-0028, ADR-0035

## Context
ADR-0010 made every edit of a published lecture or assignment description visible to students at
once, and kept draft revisions of published content out of v1, naming "the two-slot approach from
ADR-0009" as the way to add them later. A teacher who fixes a typo in a published lecture has no
way to prepare a larger rewrite without showing it half done. The web app has a "changes not yet
published" state for this (the lecture editor, the course content list, the teacher home).

The same screens need a few facts the content tree does not carry: how long a lecture takes to read,
the day an in-class piece of work takes place (a `no_submission` assignment has no deadline), what a
student's own progress on each assignment is, and what a teacher still has to grade. The gradebook
drawer needs the recorded changes of one grade, which `gradebook.grade_history` holds since ADR-0028.

## Options
- Where the published copy lives: (a) a table of revisions, one row per publish, which every student
  read must join; (b) **two slots on the item row**: the working title and body that staff edit, and
  the published title and body that students read. No join, no history to purge on delete.
- What a revision covers: (a) everything on the item; (b) **title and body only**. An assignment's
  `config` and settings (points, deadline, attempts, questions) apply at once: a student must be held
  to the deadline and the points the teacher sees, and the structure of an assignment is frozen after
  the first attempt anyway (ADR-0023).
- A day without a time for in-class work: (a) a `timestamptz` at noon, which invents a time; (b) **a
  `date`**.

## Decision
1. **Slots.** `content.items` gets `published_title`, `published_body` and `last_published_at`, null
   while the item has never been published and set (a CHECK) once its status is `published`. Staff read
   and write the working `title` and `body`; a student reads the published ones, everywhere:
   the tree, `GET /items/{id}`, the grades of `my-grades`. An item's `has_unpublished_changes` is that
   its working pair differs from its published pair.
2. **Publishing.** Publishing an item, or the draft items of a module, copies the working slot into the
   published one. Publishing a module with `cascade` does not publish the edits of items that were
   published already. `POST /items/{id}/revision/publish` (`EditContent`, audited for an admin as
   `content.item.revision_publish`) copies the working slot of a published item; for a draft it is
   `409` ("publish the item first"). It answers the revision, and publishing again with nothing new
   changes only `last_published_at`. `GET /items/{id}/revision` (`ManageContent`) answers
   `{has_unpublished_changes, last_published_at, size_bytes}`, the size being that of the working body.
3. **Existing items.** The migration copies title and body into the published slot of every published
   item, with `last_published_at` set to its last change.
4. **Class date.** `content.assignments.class_on date`, allowed only for `no_submission` (a CHECK and
   `422` on `class_on`). It is set when an assignment is created and with `PATCH /items/{id}`
   (`null` clears), and an assignment carries it.
5. **Item meta.** `GET /courses/{id}/item-meta` (anyone who may view the course) answers
   `{items: {item_id: facts}}` for the items the caller may see. Everyone gets `read_minutes` for a
   lecture (words of the visible body divided by 200, rounded up, at least 1) and `class_on`. A student
   also gets, for each assignment, `status`, `score` (published only), `draft_saved_at`, `published_at`
   and `attempts_used`. Staff also get `edited` and, for each assignment, `submitted`, `to_grade`,
   `draft_grades` and `graded_total` over the current students.
6. **The status of a student's work** is one rule, kept in `content` for the item facts here and for the
   home screens later: `graded` when a published value exists; else `in_class` for `no_submission`; else
   `submitted` when an attempt exists; else `draft` when a draft exists; else `missed` when the window of
   ADR-0035 point 5 has closed; else `not_started`.
7. **Grade history.** `GET /assignments/{id}/grades/{student_id}/history` (`ViewGradebook`) lists the rows
   of `gradebook.grade_history` newest first, paged by id: the action (`save_draft` or `publish`), the
   score and feedback after it, the person (`null` if there is none) and the time. It is data, not
   sentences: the client words it.
8. The read models of 5 and 6 join `content` with `assessment`, `gradebook` and `teaching`, as the
   read models of ADR-0015 do; nothing writes outside `content`.

## Consequences
- ADR-0010's sentence "Edits to already published content are visible immediately" no longer holds for
  titles and bodies; ARCHITECTURE.md and the tests that pinned it are changed.
- Staff see the working copy everywhere, so they see an edit that students do not. The tree shows
  `has_unpublished_changes`-style information only through `GET /courses/{id}/item-meta` (`edited`).
- A teacher cannot discard unpublished changes (go back to the published text) yet; it is one more
  endpoint when someone asks.
- The published slot doubles the storage of a published item's body (2 MiB at most, ADR-0025).
- Search, `/me/work` and the course cards read the published slot for students.
- Deadlines and points are not revisioned: changing a deadline shows to students at once.
