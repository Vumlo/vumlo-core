# 0009. Grading: draft/published slots, gradebook = assignments

Status: Accepted

## Context
Teachers grade privately and publish all grades at once. They also need to fix an already published grade without the student seeing intermediate states. Schools give grades without any digital submission (oral answers).

## Options
1. `status: draft | published` on a grade — editing a published grade either leaks immediately or hides the grade.
2. **Two slots per grade** — draft and published values side by side.

## Decision
- `grades` keyed by `(assignment_id, student_id)` with `draft_*` and `published_*` columns.
- Grading writes only `draft_*`. "Publish" (per assignment or selected students) copies draft → published in one transaction and emits one `grade.published` per student.
- `grade_history` is an append-only log of every save and publish.
- Students only ever receive `published_*`. A dedicated repo function and a test enforce this.
- Teachers and assistants can both grade and publish.
- **The gradebook has no separate tree.** Every column is an assignment in the course tree. "+ column" in the gradebook creates a `no_submission` assignment in a module the teacher picks. Column order = tree order.
- Final grades are out of scope for v1.

## Consequences
- The UI can show "unpublished changes" when draft ≠ published.
- Schools should use one `no_submission` assignment per lesson, not per oral answer, to avoid clutter.
