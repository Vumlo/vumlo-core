# 0048. Course info, archiving, deletion, templates and the people of a course

Status: Accepted
Supplements: ADR-0004, ADR-0005, ADR-0019, ADR-0021, ADR-0035, ADR-0037
Amends: ADR-0033 (decision 2: course editing and roster writes no longer ignore the status),
ADR-0030 (the course status now matters to writes)

## Context
`teaching.courses.status` has had the value `archived` since ADR-0019, but nothing sets it, and the
only effect it has is accidental: a student loses every right in an archived course, while staff can
still change everything. ADR-0019 left archiving "to a later ADR". ADR-0005 planned templates as
courses of `kind = 'template'` and the deep copy, which does not exist. ADR-0021 and ADR-0035 say a
course delete must lock the assignment rows and call `gradebook` and `assessment`; nothing deletes a
course. The web app also needs a course's code and description, counts, the names of the people on it,
and a way to find people to add.

## Options
- What archived means: (a) hidden from students, writable by staff (today, by accident); (b) **visible
  and read-only for everyone**: a finished course stays readable by its students and nobody can change
  a grade or a lecture by mistake.
- Where "read-only" is decided: (a) in each service function; (b) **in `policy::authorize`**: a write
  action on an archived course answers `Archived`, one `409`, in every module. Actions that serve both
  reading and writing are split so that only the writing half is refused.
- What `restore` returns to: (a) the status before archiving, which needs a column; (b) **`published`**.
- A copy of a course: (a) one SQL statement with a CTE that maps old ids to new ones; (b) a loop in
  Rust, one insert per row. Chosen: (a), one statement and no half-copied tree.
- People of a course: (a) names and emails to everyone on the course; (b) **names to everyone,
  emails only to staff**.

## Decision
1. **Columns.** `teaching.courses.code` (text, trimmed, at most 32 characters, default empty) and
   `description` (text, at most 300 characters, default empty). They are edited with
   `PATCH /courses/{id}/info {code?, description?}` (`EditCourse`, audited for an admin) and read with
   `GET /courses/{id}/info`. `POST /courses` and the `Course` object are unchanged.
2. **Archiving.** `POST /courses/{id}/archive` and `POST /courses/{id}/restore` are the action
   `ArchiveCourse`, a root action of the owner, a course teacher and admins (ADR-0004), audited for an
   admin (`course.archive`, `course.restore`). Archiving a draft or a published course sets `archived`;
   archiving an archived one changes nothing. Restoring sets `published` (also for a course that was a
   draft before: archiving forgets that), and restoring a course that is not archived changes nothing.
   Both answer the `Course`.
3. **What archived does.** `PolicyError` becomes `Denied | Archived`. These write actions answer
   `Archived` on an archived course, for everyone, admins included: `ManageMembership`, `EditCourse`,
   `GradeAssignment`, `PublishGrades`, `SubmitWork`, and the new `EditContent` (writing content),
   `ChangeRoster` (restoring an exclusion) and `ChangeGroupLinks` (linking and unlinking a group). The
   old `ManageContent`, `ManageRoster` and `LinkGroups` keep their rules and now serve only reading
   (drafts in the content tree, the lists of links and exclusions, the group options). `Archived` is
   `409` ("the course is archived"). Reads are unchanged. A student of an archived course reads it as
   they would a published one (`ViewCourse`, `ViewContent`, `ViewOwnGrades`, `ViewOwnSubmission`); a
   student's submit or draft save is `409`. Still allowed on an archived course: restore, delete,
   transfer ownership and everything that only reads, including copying it as a template.
   `Archived` is decided after `Denied`: someone who may not do the action is told so, not that the
   course is archived.
4. **Deletion.** `DELETE /courses/{id}` is `DeleteCourse` (the owner and admins), in every status,
   audited for an admin (`course.delete`). In one transaction it locks the course row, locks the
   course's assignment rows `FOR UPDATE` (`content::lock_course_assignments`, ADR-0035 point 12), and
   asks `gradebook::has_grades_in_course` and `assessment::has_submitted_in_course` in later
   statements: a value in any draft or published slot, or a submitted attempt, is `409` (archive the
   course instead). Otherwise it purges the empty grade rows, their history and the drafts
   (`gradebook::purge_course`, `assessment::purge_course`) and deletes the course; its tree,
   memberships, links and exclusions go with it by `ON DELETE CASCADE`. The answer is `204`.
5. **Templates** (ADR-0005). `POST /courses/{id}/template {title?}` makes a `template` course owned by
   the caller from any course they can edit the content of; `POST /templates/{id}/courses {title?,
   term_id?}` makes a draft `regular` course owned by the caller from a template they can edit the
   content of. Both need `CreateCourse` and answer `201` with the new `Course`. `GET /me/templates`
   lists the templates the caller owns or teaches with the numbers of sections, lectures and
   assignments and the time they were last saved. The copy is `content::copy_course_content`, one
   statement with a CTE that gives every module and item a new id. It copies the tree, the bodies and
   each assignment's type, `config`, `max_points`, `is_bonus` and `max_attempts`. Everything copied
   is a draft with no `publish_at`, and no deadline, late rule or class date is copied. Members,
   groups, exclusions, submissions and grades are not copied. `code` and `description` are copied.
6. **Info** (`GET /courses/{id}/info`, anyone who may view the course): code, description, the term's
   name, the owner's name and email, the numbers of current students, former students (ADR-0037) and
   assistants (the owner's address goes to everyone who can view the course: the owner is the person
   to ask), the time the course row last changed and the time content last changed, the sum
   of `max_points` of the regular assignments the caller may see, their number, and `has_grades`
   (whether the course can be deleted). It is a read model: `teaching` reads the tables of
   `identity`, `org` and `content` with joins and asks `gradebook` for `has_grades`.
7. **People** (`GET /courses/{id}/people`, anyone who may view the course): everyone on the roster
   with name, surname, role, `is_owner`, `source` and, for a student from a linked group, the group's
   (or subgroup's) name. Only staff get the emails, the former students (ADR-0037) and the list of
   linked groups with their student counts. A student sees classmates' names and nobody's email.
   **Candidates** (`GET /courses/{id}/candidates?q=`, `ManageRoster`): `q` has at least 2 characters
   (`422` otherwise); active users who are not on the roster, matched case-insensitively on name,
   surname and email with the pattern characters escaped, ordered by surname, name and id, at most 20.
   It reveals names and emails of the whole institution to everyone who manages a roster, assistants
   included; ADR-0040 already limits who can link a group, and this does not widen it.
8. **Dependencies.** `teaching` calls `content`, `gradebook` and `assessment` for the delete guards and
   the purge, and `content` calls `teaching` for the access; the cycle is inside one crate and is
   recorded here, like those of ADR-0030 and ADR-0035.

## Consequences
- ADR-0033 decision 2 and `ManageContent`'s "ignores the course status" (ARCHITECTURE.md) are narrowed:
  writing content, grades, members, links and settings stops at an archived course.
- The tests `students_cannot_see_an_archived_course_but_staff_can` and
  `content_of_an_archived_course_is_editable_by_staff` described the accidental behavior and are
  rewritten to the new rule; the policy tests that listed `EditCourse` and the roster writes as
  status-blind now say `Archived`.
- A restored course is published even if it was a draft, and there is no way back to `draft`
  (ADR-0019 left unpublishing open).
- A course with a graded value, however old, can only be archived. A course made by mistake and graded
  can only be archived too.
- Auto-archiving at the end of a term, `left_on` of former students and a template "update from
  template" are not decided here.
