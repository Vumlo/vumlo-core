# 0028. Gradebook: grades, publishing, read models

Status: Accepted
Supplements: ADR-0009, ADR-0015, ADR-0021, ADR-0023

## Context
ADR-0009 fixed the two slots of a grade, ADR-0015 made the gradebook a module and ADR-0021 listed the callbacks `content` owes it. They leave open what a slot holds, what retracting a published grade means, what publishing copies, what the two read models show, and how the delete and lowering guards work. ADR-0023 left one question to this task: whether students see `no_submission` items in the course tree. The first event of ADR-0007 is emitted here.

## Options
Retraction: (a) **publishing an empty draft retracts the published value**; (b) a separate "unpublish" action: one more endpoint and state for a case that is rare in a school.

Serializing a grade write with a change of `max_points`: (a) the advisory lock of ADR-0022: it does not stop a delete; (b) **a row lock on the assignment row in `content.assignments`**: the grade write takes `FOR SHARE`, the `max_points` update and the deletes take `FOR UPDATE`, so one lock covers both; (c) a course-level lock: queues unrelated writers.

## Decision
1. A grade is one row per `(assignment, student)` with a draft slot and a published slot (ADR-0009). Each slot holds `score numeric(6,2)` (nullable) and `feedback` (nullable plain text, trimmed, at most 4000 characters; empty after trimming means null). Zero is a valid score. A score satisfies `0 <= score <= max_points` of the assignment (ADR-0021). The upper bound is checked in the gradebook service, not in a CHECK.
2. A cell is "published" for the student only when its published score or published feedback is non-null. Clearing a draft and then publishing it retracts the published value: both published fields become null and the student sees nothing. A retraction emits no event in v1. `has_unpublished_changes` is true when the draft pair differs from the published pair.
3. Publishing copies draft to published for all students or for a listed set, in one transaction, only for rows whose draft differs from the published value. It emits one `grade.published` for each student whose published value became non-empty and changed; the payload holds ids only (ADR-0007). Saving a draft emits no event.
4. `gradebook.grade_history` is append-only: one row per draft save and per publish (who, when, action, the values after the action). Nothing updates or deletes a history row except the purge of point 9.
5. Concurrent draft writes to one cell are last-write-wins; the history keeps both. Publishing locks the rows it copies. A grade write, a change of `max_points` and a delete on the same assignment are serialized through the assignment row in `content.assignments`: the grade write and the publish take `FOR SHARE` and read `max_points` from the locked row, the `max_points` update and the deletes take `FOR UPDATE` first and call the gradebook in a later statement.
6. Read models. (a) The staff gradebook of one course: the columns are every assignment of the course in tree order, drafts included, with title, `max_points`, `is_bonus` and item status. The rows are the current effective student members of the course (ADR-0006, a read-only JOIN with `teaching`), sorted by last name, first name and id, and paginated by student (ADR-0018). A student who left keeps their rows in the database and is not listed. (b) "My grades" for a student: only published values, only for assignments visible to that student under ADR-0010. A dedicated repo function selects only the `published_*` columns; no student-facing query selects a `draft_*` column.
7. Students see a `no_submission` item in the course tree under the ordinary publication rules of ADR-0010.
8. Dependency direction: `content` calls public gradebook functions (the highest score, the has-grades checks, the purge). `gradebook` reads `content`, `teaching` and `identity` tables only through read-only SQL JOINs and never calls their Rust functions. A row lock (`FOR SHARE`) on `content.assignments` is a lock, not a write.
9. Guards from ADR-0021, gradebook half. Lowering `max_points` below any existing draft or published score of the assignment is refused with 409. Deleting an assignment, a module (with its sub-modules) or a course that has any grade with a non-empty draft or published value is refused with 409. Rows whose values are all empty do not block a delete; the delete path purges them and their history in the same transaction, so no row is left behind without an assignment.
10. Not in this task: final grades, categories and weights, points to scale conversion, export, batch grade writes, a history endpoint, the submission status in a cell (task 013), "absent" marks.

## Consequences
- The callbacks that ADR-0021 asked of the gradebook exist. A course delete does not exist yet; the task that adds it must call `has_grades_in_course` and then `purge_course` before it deletes.
- `teaching.effective_members` is not built yet. Until it is, the roster is the direct student memberships, and the group-links task repoints that one JOIN.
- The sort by last name and first name needs those columns in `identity`.
- The gradebook cannot call `teaching::course_access`; it reads the actor's course role with its own query and passes it to `policy`.
- `grade.published` is the first row of `events.outbox`. Delivery is the worker task's job.
- A grade has no foreign key to its assignment (ADR-0021: another schema). The guards and the purge keep the two consistent.
- Row locks on `content.assignments` make a grade write wait for a slow `max_points` update or delete on the same assignment, and the other way round.
