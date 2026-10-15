# 0008. Extensible assignment types

Status: Accepted

## Context
Assignments differ: file upload, text answer, forms with questions, code, no submission at all (oral answers, class work, paper tests). New types will keep appearing, and assignment types are the main extension point of an LMS.

## Decision
- `assignments.type` (text) + `assignments.config` (jsonb). `submissions.content` (jsonb) is shaped by the type.
- In code a type is a Rust enum (`serde` tagged) implementing a trait such as `AssignmentType { validate_config, validate_submission, auto_grade? }`. Every JSON written to the DB goes through it.
- `text` in the DB, not a Postgres `ENUM`, so a new type needs no DB migration.
- The type cannot change once submissions exist.
- v1 types are built in: `file`, `text`, `form` (questions; can be auto-graded), `no_submission` (oral answers, class work, paper tests — graded by hand). A registry where plugins add types comes later with the extension model.

## Consequences
- New types are added in one place.
- The trait is the future plugin boundary (WASM / external tool), so it must stay small and stable.
