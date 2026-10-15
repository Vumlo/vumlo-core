# 0006. Groups (cohorts), live links, course subgroups

Status: Accepted

## Context
Students belong to institution-wide groups (school class 7-A, university group PZ-31) that take many courses together. A group can be split into subgroups (7-A English group 1 and 2) that take some courses separately. Inside a course there are also local splits that exist only there (lab group 1/2).

## Decision
- **Groups** (`org.groups`) and their **subgroups** (`org.subgroups`, exactly one level, no deeper nesting) are managed by admins, manually or by CSV import. A subgroup member must also be a member of its group.
- A course links a whole group or one of its subgroups through `course_groups (course_id, group_id, subgroup_id?)`. The link is **live**: `effective_members` is a view = direct memberships ∪ students of linked groups / subgroups.
- Teachers and assistants are always direct memberships.
- **Course subgroups** (`teaching.course_subgroups`) are flat, local to one course, and never synced with institution groups.
- Grades and submissions are keyed by `user_id`. A student who leaves a group loses access but keeps their history.

## Consequences
- Adding a student to 7-A enrolls them in all of 7-A's courses at once, with no sync job.
- Membership checks join through a view. Fine at institution scale; revisit if profiling says otherwise.
