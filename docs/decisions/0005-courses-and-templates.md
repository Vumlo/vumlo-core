# 0005. Courses and templates in one table, copy on create

Status: Accepted

## Context
Schools teach the same subject to several classes at different paces (Algebra 7-A, 7-B). Universities rerun the same course every year. Teachers also create courses from scratch.

## Options
1. Separate template and course entities, content linked live with overrides — powerful, complex sync.
2. **One `courses` table with `kind = regular | template`, content copied** — simple.

## Decision
- `courses.kind = regular | template`. Templates have no term and no students.
- "Create course" = empty regular course. "Save as template" and "Create from template" = deep copy of the content tree.
- Content always belongs to exactly one course. Nothing is shared live.

## Consequences
- A change to a template does not reach existing courses. "Update from template" can be added later.
- One deep-copy function to write and test well.
