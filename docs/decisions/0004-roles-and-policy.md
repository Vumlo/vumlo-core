# 0004. Two-level roles and a central policy

Status: Accepted

## Context
A person's rights depend on who they are in the institution and on who they are in a specific course. Examples: a student can be a teaching assistant in one course; a teacher can join another course as a student; an admin can act in any course.

## Options
1. Fixed roles, mapping in code.
2. Full RBAC configurable in the database/UI (Moodle style) — flexible, but very complex.

## Decision
- Institution role `users.role`: `admin | teacher | student` (`guardian` later).
- Course role `memberships.role`: `teacher | assistant | student`, plus `courses.owner_id`.
- Course role `teacher` requires institution role `teacher` or `admin`.
- Assistants have full course access **except root actions**: add/remove teachers and assistants, change the grading scale, archive, delete, transfer ownership.
- Teachers can do root actions except delete and transfer ownership, which are owner/admin only.
- An admin has no memberships, acts as an implicit owner everywhere, and **every admin action in a course is audited**.
- The role → permission mapping lives in code, in the `policy` module. Every check goes through `authorize(actor, action, resource)`.
- The full matrix is in `ARCHITECTURE.md`.

## Consequences
- One place to read and test permissions.
- Custom roles per institution need a new ADR.
