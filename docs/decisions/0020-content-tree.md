# 0020. Content tree: course_id on every row, one level of sub-modules, dense positions, opaque lesson body

Status: Accepted
Supplements: ADR-0005, ADR-0010

## Context
A course page shows sections that contain sub-sections (for example "Lectures" and "Practice sessions") and also items placed directly in a section (for example "Homework 1"). Access to any content row depends on its course (membership, course status). Reaching the course from an item takes two joins, and three from a submission, on every request and every policy check. Modules and items also need a stable order the teacher can change, and lessons need a body whose format depends on the editor, which is not chosen yet (ADR-0013).

## Options
Depth: (a) flat modules — cannot express sub-sections; (b) **modules with one level of sub-modules** — covers the real pages, same "one level only" rule as groups (ADR-0006); (c) arbitrary depth — every visibility, cascade and UI rule becomes recursive.

Course reference: (a) derive the course by joining item → module → course — fully normalized, but every access check pays the joins; (b) **`course_id` on modules and items, kept consistent by composite foreign keys** — one extra column, one cheap check.

Order: (a) fractional keys — no renumbering, but unreadable values and a helper to write; (b) **dense integers renumbered by a full-list reorder** — simple, fine for trees of a few hundred rows.

Lesson body: (a) markdown text — fixes the editor choice early; (b) **opaque jsonb object** until the frontend ADR.

## Decision
- `content.modules.course_id` references `teaching.courses` (ON DELETE CASCADE) and `modules` has `UNIQUE (id, course_id)`.
- `modules.parent_id` is NULL for a top-level module. A sub-module's parent must be a top-level module of the same course: `FOREIGN KEY (parent_id, course_id) REFERENCES content.modules (id, course_id) ON DELETE CASCADE`. A sub-module cannot have sub-modules (depth is at most one). The database enforces this if a clean constraint exists, and the service checks it either way (422).
- `content.items` carries `course_id` too, with `FOREIGN KEY (module_id, course_id) REFERENCES content.modules (id, course_id) ON DELETE CASCADE`. An item lives in a top-level module or in a sub-module, never in two places. `course_id` and `parent_id` never change in this version: content belongs to exactly one course (ADR-0005), and moving is a later task.
- Assignment rows (later task) get their course through `item_id` and do not repeat `course_id`.
- `position` is an integer >= 1. Top-level modules are one sequence per course; sub-modules are one sequence per parent; items are one sequence per module. Sub-modules and items of the same parent are two separate sequences, and clients show sub-modules first, then direct items. All uniqueness constraints are `DEFERRABLE INITIALLY DEFERRED`. A new row goes to `max(position) + 1` in its sequence; gaps after a delete are allowed. A reorder is a PUT with the complete ordered list of one sequence's ids; it must be exactly the current set (422 otherwise) and is rewritten in one transaction. A concurrent conflicting write violates the constraint and returns 409; the client retries. No locks on parent rows.
- A lesson `body` is a JSON object (default `{}`) that the server stores and returns without interpreting it. Its format is decided with the frontend. Size is bounded by the global request body limit.
- A student sees an item (ADR-0010) only if the course, every module on the path from the top-level module to the item, and the item are `published` and each of their `publish_at` is null or in the past. This is computed in the SQL of list queries and checked again by `policy` for every row, as for courses. `policy` has no clock: the evaluation time travels inside the `Action`.
- Publishing with `cascade = true` publishes the module, its sub-modules and every draft item below them, in one transaction.

## Consequences
- An item or a module cannot move to another parent or course in this version; copy it instead (ADR-0005), or add a move task.
- Reordering sends the whole list, which is fine for course-sized trees.
- The server cannot search or render lesson bodies until the format is chosen.
- A new content table must carry `course_id` too, or reach it through a table that does.
