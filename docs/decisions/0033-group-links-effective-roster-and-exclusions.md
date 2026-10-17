# 0033. Group links, the effective roster and course exclusions

Status: Accepted
Supplements: ADR-0004, ADR-0006

## Context
ADR-0006 says a course links a group or a subgroup, that the link is live, and that `effective_members` is a view. Task 014 (ADR-0032) built groups and subgroups, but nothing uses them yet: the view is still the direct memberships, and `teaching` reads roles from `teaching.memberships` in several places. A teacher also needs to take one person out of one course without touching the institution group, and to correct the title or the term of a course.

## Options
- Precedence on the roster: (a) **direct membership, then exclusion, then group**; (b) an exclusion beats a direct membership. (b) would let an old exclusion overrule a teacher who just added the person by hand.
- How a link to a subgroup is kept valid: (a) the service checks; (b) **a composite foreign key** `(subgroup_id, group_id)` to `org.subgroups (id, group_id)`. The key `UNIQUE (id, group_id)` already exists (migration 0014), so no table of `org` changes.
- "Would this person still be a student through a group?" when a direct student is removed: (a) repeat the group SQL in a second query; (b) **a helper view `teaching.group_roster`** that `effective_members` is built on.
- Reading `org`'s membership tables from the view: (a) **`org.group_members` and `org.subgroup_members` are declared as the read interface of `org` for membership; Rust code outside `org` never reads them (an architecture test checks it)**; (b) a view owned by `org`, which adds machinery for the same columns.
- How staff choose a group to link: (a) open group reads to teachers; (b) **a course-scoped read for those who may manage the roster**, ids and names only. `org`'s own group reads stay admin only.

## Decision
1. `teaching.course_groups` is a live link of a course to a whole group, or to one subgroup of it. Deleting the course deletes its links. The group and the subgroup are protected by `ON DELETE RESTRICT` foreign keys; `org` maps the violation (SQLSTATE `23001`) to `409` when a group or a subgroup is deleted. A subgroup must belong to the linked group (the composite key). Linking the same target twice is idempotent and returns the existing link. A whole-group link and a link to one of its subgroups may coexist; the roster deduplicates.
2. The roster is `teaching.effective_members (course_id, user_id, role, source)`, one row per (course, user). A direct membership wins: its role, source `direct`. Otherwise a user who is a member of a linked group, or of a linked subgroup, is a `student` with source `group`, unless an exclusion exists for (course, user). The columns `course_id`, `user_id` and `role` keep their meaning; `source` is appended.
3. Exclusions are `teaching.course_exclusions (course_id, user_id, created_by, created_at)`. Removing a person from the students of a course deletes their direct student membership if they have one and, if they would still be a student through a group, records an exclusion. Removing a teacher or an assistant deletes the direct row only and never records an exclusion; if the person is also in a linked group they remain a student of the course through it. Adding a direct membership removes an exclusion of that user. An exclusion stays until it is restored, also when the user later leaves the group, and is then inert. Exclusions never touch grades or submissions; they only change who is on the roster (ADR-0006).
4. Everything in `teaching` that reads a member's role reads the view: `course_access`, the `ViewCourse` decision, "my courses", the member list, and the membership writes that check the current role. Direct membership writes still go to `teaching.memberships`. No module outside `teaching` reads `teaching.memberships`, `course_groups`, `course_exclusions` or `group_roster`, and none outside `org` reads `org.group_members` or `org.subgroup_members` (architecture test).
5. Staff choose a group through `GET /courses/{id}/group-options`: groups with their subgroups (ids and names, no members, no counts). It is answered by a function of `org` that takes no `Actor` and that the caller has authorized, as `identity::find_user` is.
6. `PATCH /courses/{id}` changes `title` and `term_id`. Editing metadata is not a root action (ADR-0004), so assistants, teachers, owners and admins may do it and students may not; the course status does not matter. A template cannot have a term (`422` on `term_id`); an unknown term is `422` on `term_id`; the title rules are those of creation. `term_id: null` clears the term, an absent field changes nothing. Admin edits are audited.
7. Links, exclusions and the roster writes are allowed to those of the matrix row "Enroll/remove students, link groups, manage course subgroups": assistant, teacher, owner, admin. Admin writes are audited. Member, link and exclusion writes lock the course row first, like the existing member writes, and read roles in a later statement.

## Consequences
- A student added to a linked group sees the published course at once; leaving the group takes the access away. Their grades stay, because grades are keyed by `user_id`.
- A group or a subgroup that a course uses cannot be deleted until the link is removed.
- An exclusion can become stale (the user left the group) and then does nothing; if the user is added to the group again they are still excluded until it is restored.
- Removing an assistant or a teacher who is also in a linked group leaves them a student of the course.
- A group student put on the course as `student` already holds that role, so the call changes nothing.
- The yearly rollover stays manual (ADR-0032): rename the group and change its members, and the linked courses follow.
- `teaching` now depends on `org` (one way). Teachers still cannot read groups through `org`; they pick one through the options read.
- Course subgroups and CSV import are not part of this decision.
