# 0032. Terms, groups and subgroups

Status: Accepted
Supplements: ADR-0005, ADR-0006

## Context
ADR-0006 names groups and subgroups but fixes no shape, and ADR-0005 says a regular course has a term without saying what a term is. `teaching.courses.term_id` has no foreign key (migration 0004 announced one). Task 015 links groups to courses and needs these tables to exist.

## Options
- Term shape: (a) **flat**; (b) a school year with semesters inside it. (b) is decided together with final grades.
- "A term is in use": (a) **a foreign key `ON DELETE RESTRICT`**; (b) a query into `teaching` before the delete, which is a cross-module read used to guard a write.
- The rule "a subgroup member is a member of the group": (a) the service alone; (b) **the service under a group-row lock, with a composite foreign key as the backstop**, the way ADR-0020 ties content rows to their course.
- Who reads groups: admins only for now. How a teacher or an assistant picks a group for a course is decided in task 015.

## Decision
1. Terms are flat: `id` (UUID v7), `name` (trimmed, 1–100 characters, unique), `starts_on` and `ends_on` (dates, `ends_on >= starts_on`). No hierarchy now.
2. Groups have `id`, `name` (trimmed, 1–100, unique) and `created_at`. Subgroups have `id`, `group_id` and `name` (trimmed, 1–100, unique inside the group), one level only. Renaming is allowed. The yearly rollover of school classes is manual in v1: rename the group and change its members.
3. Group members are users whose institution role is `student`. A subgroup member must be a member of its group. Removing a user from a group removes them from that group's subgroups in the same transaction. A later role change of a user does not remove memberships. Disabled users may be members.
4. Member writes lock the group row first, as member writes lock the course row in `teaching`, so two concurrent writers see each other's committed result. The lock statement returns the group row only; memberships are read in a later statement.
5. Creating, changing and deleting terms, groups, subgroups and members is admin only. Reading terms is open to any signed-in user. Reading groups, subgroups and members is admin only in this task.
6. Deleting a term that a course uses is refused with 409. It is enforced by the foreign key `teaching.courses.term_id -> org.terms ON DELETE RESTRICT` and the violation is mapped to 409; no module reads another's tables for it. Deleting a group deletes its subgroups and members. Task 015 adds the same `RESTRICT` key from the group links, so deleting a group that a course uses becomes 409; until then nothing links to a group.
7. Admin writes in `org` are written to `events.audit_log` in the same transaction, with the resource types `term`, `group` and `subgroup`. Reads are not audited.
8. No events are emitted.
9. The subgroup invariant is also held by the database: `subgroup_members (group_id, user_id)` references `group_members` with `ON DELETE CASCADE`. Removing a user from a group deletes their subgroup rows in the same statement.
10. Every write below a group (its members, creating, renaming and deleting its subgroups, subgroup members) takes the group-row lock first. A write addressed by a subgroup id reads `group_id` of the subgroup, locks that group, then reads the subgroup again.
11. "Only a student joins a group" is a function of `policy` (`group_role_allowed`), as `course_role_allowed` is for courses.
12. The member lists read the name and surname from `identity.users` with a read-only JOIN to sort by surname, name and id, as the gradebook roster does (ADR-0028). The role check when adding goes through `identity::find_users`.
13. Names are trimmed, contain no control characters, and are unique by exact text (case-sensitive).
14. `POST /courses` accepts an optional `term_id`, and the course carries it. A term that does not exist is `422` on the field `term_id`. A course update endpoint does not exist and is not added here.

## Consequences
- A term in use cannot be deleted; delete or move its courses first (moving needs a course update, which is not built).
- A group that a course uses will be undeletable once task 015 lands; until then deleting a group cascades.
- The rollover is manual; there is no copy-to-next-year action.
- Teachers and assistants cannot read groups yet. Task 015 must say in its ADR how they pick one.
- `identity` gets a read function for several users at once.
