# 0021. Assignments and scores

Status: Accepted
Amends: ADR-0008
Supplements: ADR-0009, ADR-0020

## Context
The content tree (ADR-0020) holds lessons only. Assignments are the items students answer and teachers grade. ADR-0008 fixed extensible assignment types and ADR-0009 the draft/published grade slots, but not the shape of the assignment row, the type and range of points, what may change once grading has started, or which types ship first.

## Options
Points type: (a) float — 0.1 + 0.2 errors in a gradebook; (b) integer — no half points; (c) **`numeric(6,2)`** — exact, handled as `rust_decimal` (ADR-0014).

Where max points live: (a) inside the type's `config` — every type repeats it and totals cannot be summed in SQL; (b) **a column on `assignments`** — every graded thing has it, including the `no_submission` gradebook columns of ADR-0009.

Columns: (a) every column ARCHITECTURE.md lists (due date, late rules, attempts) now — unused and unreviewed until submissions exist; (b) **only columns that have behavior in the same task** — later tasks add the others with their own migration.

Changing the type: (a) allowed until submissions exist (ADR-0008) — needs a check against another module; (b) **fixed at creation** — simpler, and relaxing it later is additive.

## Decision
- `content.assignments` has `item_id uuid PRIMARY KEY` referencing `content.items (id) ON DELETE CASCADE`, `type text NOT NULL`, `config jsonb NOT NULL DEFAULT '{}'` (a JSON object), `max_points numeric(6,2) NOT NULL CHECK (max_points > 0)`, `is_bonus boolean NOT NULL DEFAULT false` (extra credit, see ADR-0022), `created_at` and `updated_at`. An item of kind `assignment` has exactly one row; a `lesson` has none. The service creates the item and its row in one transaction, and the database enforces that a row belongs to an item of kind `assignment` if a clean constraint exists (the same pattern as the depth rule in ADR-0020).
- `type` is `text` with no CHECK (ADR-0008) and is validated in code through the `AssignmentType` trait. Types are added one per task; the first is `text`. The type is chosen when the assignment is created and cannot change in this version (stricter than ADR-0008; relaxing it later is additive). `config` is validated by the type, and unknown fields are rejected.
- Points are `numeric(6,2)`. `max_points` is greater than 0 and at most 9999.99. In the API a point value is a decimal string with at most two decimals ("20", "7.5" and "20.00" are accepted); responses always carry two decimals.
- A score is valid when `0 <= score <= max_points`. The gradebook checks this in its service: grades live in another schema, and no CHECK can span schemas. Zero is a valid score; the grading scale limits `max_points`, not the lowest score (ADR-0022).
- `max_points` may be raised at any time. Lowering it below any existing draft or published score of that assignment is refused with 409. `content` enforces this by calling a public gradebook function that returns the highest score given. Until the gradebook exists there are no scores and nothing to check; the gradebook task must add the call.
- Deleting an assignment, or a module or course that contains one, will be refused with 409 when it has submissions or grades, through public functions of `assessment` and `gradebook`. The database cascades deletes from course to module to item to assignment (migration 0005), and grades live in another schema with no foreign key, so without this guard a delete would orphan them. Until those functions exist, deletion is allowed.
- Due dates, late rules and attempt limits (ADR-0011) are added to `content.assignments` by the task that gives them behavior.

## Consequences
- Totals and percentages can be computed in SQL.
- The gradebook and assessment tasks must write the callbacks listed above; nothing in `content` guards those cases before they exist.
- JSON clients must treat points as strings, not floats.
- Types other than `text` need their own tasks, and `form` will probably need its own ADR for the question format.
- The `gradebook` and `assessment` tasks must also guard module and course deletion: the delete code in `content` and `teaching` must call their functions.
- `max_points` is also limited by the grading scale (ADR-0022): at most 12 per assignment for `twelve`, at most 100 regular points per course for `hundred`; bonus assignments are outside that sum.
