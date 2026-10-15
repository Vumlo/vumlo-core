# 0015. Gradebook as a separate module

Status: Accepted
Amends: ADR-0001 (module list)

## Context
Grades were planned inside `assessment` together with submissions. But a grade often has no submission behind it (`no_submission` assignments), and the gradebook will grow: categories and weights, final grades, export.

## Options
1. Gradebook inside `assessment` — fewer modules, but student work and grading get mixed, and the module keeps growing.
2. **A separate `gradebook` module** — `assessment` = student work, `gradebook` = grades.

## Decision
- `assessment` owns `submissions` (drafts, attempts, lateness) and auto-grading of `form` assignments.
  Auto-grading writes a **draft grade** through `gradebook`'s public functions.
- `gradebook` owns `grades` and `grade_history` (schema `gradebook.*`): grading, publishing, the gradebook read model, and later categories, weights, final grades and export.
- The gradebook read model joins `content` (columns = assignments, ADR-0009), `teaching` (`effective_members`) and `assessment` (submission status shown in a cell). These are read-only cross-module JOINs.
- Gradebook columns remain assignments in the course tree (ADR-0009 unchanged).

## Consequences
- Module list: `identity`, `org`, `teaching`, `content`, `assessment`, `gradebook`, `files`, `events`, `policy`.
- `assessment` depends on `gradebook` (auto-grading). `gradebook` must not write to `assessment`.
