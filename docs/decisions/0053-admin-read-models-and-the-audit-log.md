# 0053. Admin read models and reading the audit log

Status: Accepted
Supplements: ADR-0015, ADR-0032, ADR-0034, ADR-0044, ADR-0048, ADR-0052

## Context
The administrator screens show more than the lists the API gives: how many people there are in each
role, what a person does in the institution, who is in a group and which courses use it, how many courses
each term has, every course of the institution, and what has been done (the audit log, written since
ADR-0034 and never readable). The web app had mocks that wrote English sentences for all of it.

## Options
- **Sentences.** (a) The server writes "gave CS 201 to Dr. Voss". (b) **It sends the action, the ids,
  the labels of the things acted on and the metadata; the client words each action** (ADR-0043).
- **Who composes a person's facts.** (a) `identity` reads the tables of `org` and `teaching`: forbidden for
  group members (the architecture test). (b) **`identity` calls one public read function of each of
  `org` and `teaching`** and puts the answers together.
- **Kinds of audit entry.** (a) A column filled when the row is written: a migration and a rule every
  writer must follow. (b) **Derived from the action when it is read**, in one place.

## Decision
1. **Who.** All of them need an administrator: `Action::ViewAdminReports` for the people summary, the facts,
   the course list, the term statistics and the audit log, and the existing `ViewGroups` for the group
   overview. Others get `403`. None is audited (reads).
2. **`GET /admin/users/summary`** (`identity`): `{all, student, teacher, admin, disabled}`: people by
   institution role (all statuses), and how many are disabled.
3. **`GET /admin/users/{id}/facts`** (`identity`): `{role, status, created_at, last_login_at, groups: [{id, name}],
   owns: [course], teaches: [course], studies}`: the groups the person is in, the regular courses they own
   or teach or assist in (`code`, `title`), and how many published courses they study in now. `404` for an
   unknown person. No email appears here.
   The time of the last sign-in is a row of the new table `identity.last_logins` (one per person, absent
   until their next sign-in), written when a session is created by a password or a Google login. It is a
   table and not a column of `identity.users` because a login holds the user row `FOR SHARE` (ADR-0038): two
   logins of one person that both updated that row would wait for each other.
4. **`GET /admin/groups/{id}/overview`** (`org`): `{group_id, student_count, course_count, members: [{user_id,
   name, surname, email, added_at}], linked_courses: [{course_id, code, title, owner_name, owner_surname,
   linked_on}]}`. `org.group_members.added_at` (new) is the time a person was put in the group; it is null for
   those who were there before this migration. The linked courses come from `teaching`; a course that uses
   only a subgroup counts. `404` for an unknown group.
5. **`GET /admin/term-stats`** (`org`): `{items: [{term_id, courses, current, read_only}]}`: how many regular
   courses use each term, whether today in the institution's time zone falls in it, and whether it has ended
   (`read_only`: the screens treat the courses of an ended term as past).
6. **`GET /admin/courses`** (`teaching`): every regular course, paged by id (ADR-0018): `{course_id, code,
   title, status, owner_id, owner_name, owner_surname, term_id, term_name, student_count}`.
7. **`GET /admin/audit?q=&kind=&days=&cursor=&limit=`** (`events`): the rows of `events.audit_log`, newest
   first, paged by id. `days` is 1 to 365 (default 7); `kind` is one of `accounts` (`user.*`), `owners`
   (`course.owner.transfer`), `teachers` (`course.member.*`), `groups` (`group.*`, `subgroup.*`), `courses` (the other
   `course.*`), `content` (`content.*`), `grades` (`grade.*`), `terms` (`term.*`) or `other` (an unknown value
   of `kind` is `422`); `q` (at most 100 characters) matches the actor's name, the action or the label of the
   target, case-insensitively as a substring. An entry is `{id, at, source, kind, action, actor: {user_id, name,
   surname} | null, resource_type, resource_id, resource_label, meta}`. `resource_label` is what the target is
   called now (a person's name, a course's code and title, a group, a term, an item or a module), or `null`
   when it is gone. The actor is `null` for a row written by the server CLI (`source: cli`).

## Consequences
- The audit log has no retention and no index on `at`; a very long period of a busy installation scans a
  lot. An index on `at` is the upgrade when it is slow (named in the code).
- The names in an entry are those of today: a person renamed later shows the new name.
- `meta` is shown as stored (ids, roles, field names): the client words what it knows and leaves the rest.
- Not decided here: exporting the log, retention, and reading it per course for a course owner.
