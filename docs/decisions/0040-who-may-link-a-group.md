# 0040. Who may link a group to a course

Status: Accepted
Amends: ADR-0033 (decision 7)

## Context
ADR-0032 keeps the members of a group out of reach of everyone but admins, and ADR-0033 lets a course
link a group. The audit of task 014 (A-05) found that the two do not fit. Linking puts the group's
students on the roster, the roster shows their names (the gradebook, the submissions list), and
ADR-0033 gave the right to link to everyone who manages the roster, assistants included. An assistant
is often a student. A student with an assistant role in one course could choose any group of the
institution from `group-options`, link it, and read the names of its members in the gradebook, and
could put a whole cohort into a published course.

## Options
- Only admins link. Safe, and teachers cannot run their own course without an admin.
- **The owner, course teachers and admins link; assistants do not.** An assistant keeps the rest of
  the roster work.
- Teachers link only groups an admin has assigned to them. The right answer in the end, but it needs
  an owner for a group, a table and a way to assign; no ADR has asked for it.

## Decision
1. Linking and unlinking a group, and `GET /courses/{id}/group-options`, are the action
   `LinkGroups`: the course owner, a course teacher and admins, in every course status. An assistant
   is refused with `403` (the course is visible to them).
2. Listing the links of a course and the exclusions, and restoring an exclusion, stay `ManageRoster`
   (assistants, teachers, owners, admins): an assistant can still see why a student is on the roster
   and take one person off it.
3. The matrix row "Enroll/remove students, link groups" of ARCHITECTURE.md is split in two.

## Consequences
- An assistant cannot read the members of a group, or enrol a cohort, through a link.
- **Not solved:** a teacher can still link any group of the institution and read its members' names.
  That is the price of the ADR-0033 design in which any teacher chooses from all groups. Closing it
  takes group ownership (the third option) and is left to the task that gives teachers a way to ask
  for a group.
