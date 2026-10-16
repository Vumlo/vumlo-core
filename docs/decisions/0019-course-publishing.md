# 0019. Publishing a course is a root action

Status: Accepted
Amends: ADR-0004

## Context
ADR-0004 lets assistants do everything in a course except the root actions it lists, and publishing a course is not among them. Publishing makes a course visible to every enrolled student, so it belongs with the people who run the course. Task 005 introduces course `status` (`draft | published | archived`) and needs a rule.

## Options
1. Leave publishing to assistants, as ADR-0004 implies — no new rule, but an assistant can open a course to students.
2. **Publishing is a root action** (owner, course teacher, admin) — one more row in the matrix.

## Decision
- `POST /courses/{id}/publish` moves a course from `draft` to `published`. It is idempotent: publishing a published course changes nothing.
- Allowed for the course owner, a course `teacher` and an admin. Not for an assistant or a student.
- While a course is `draft`, students do not see it: every course endpoint answers 404, exactly as for a non-member. Staff (owner, teacher, assistant) and admins see every status. Adding members does not depend on the status, so students can be enrolled in a draft.
- Archiving and unpublishing are left to a later ADR.
- Publishing content (modules and items, ADR-0010) stays open to assistants.

## Consequences
- ADR-0004's matrix gets one row, "Publish course", in `ARCHITECTURE.md`.
- `policy` has a `PublishCourse` action; `ViewCourse` takes the course status into account for students.
