# 0037. Who is on a course: active students, former students, role changes, owner transfer

Status: Accepted
Amends: ADR-0004, ADR-0032 (decision 3), ADR-0033 (decision 2)

## Context
The audit of task 014 found that the roster and the roles it rests on drift away from the account
behind them:

- A-01: an admin makes the owner of a course an institution student. The account keeps the
  `teacher` membership and the ownership, and with them every staff power in the course.
- A-02: a student who sits in a group that a course links is promoted to teacher. ADR-0032 decision 3
  keeps the group membership, and the view `teaching.effective_members` then lists the teacher as a
  student of the course, in its gradebook, gradable.
- A disabled student stays on every roster, in the gradebook and in "publish all", and can be graded.
- The manual rollover (rename the group, change its members) removes last year's students from the
  course. Their results stay in the database and nobody can read them any more, not even the
  student. Changing the members of a group is one request per person.

## Options
- Roster: (a) filter in each reader; (b) **one definition, in the view `effective_members`**, so that
  every reader (access, gradebook, submissions, the member list) agrees.
- A demoted teacher: (a) refuse every demotion until an admin cleaned up by hand; (b) **refuse only
  while the user owns a course (there the admin has to choose a new owner), otherwise remove the
  teacher memberships in the same transaction**.
- Results of people who left: (a) keep them out of every list; (b) **list them to staff, read-only,
  as former entries; let the student read their own published grades**.
- Rollover: (a) a new "next year" action, which needs course copies (ADR-0005, not built); (b) **a
  bulk replace of a group's members**, so the documented procedure is one request.

## Decision
1. **The student roster.** A row of `teaching.effective_members` with role `student` exists only for
   an **active** user. A student who comes from a group must also hold the institution role
   `student`. A direct `student` membership keeps working for an institution teacher, because
   ADR-0004 lets a teacher join a course as a student; a direct membership of an institution `admin`
   cannot be written. Assistants, teachers and the owner are listed as before.
   The view reads `identity.users` (`status`, `role`): that read is declared here as an interface of
   `identity` for `teaching`, next to the group tables of ADR-0033, and the architecture test knows it.
2. **Former students.** A user who is not on the roster of a course but has a grade row with a
   value (draft or published) in it, or a submitted attempt in an assignment of it, is a *former
   student* of that course. The staff gradebook and the staff list of an assignment's submissions
   show them among the current students, in the same order (surname, name, id), each with
   `former: true` and all their data. They are
   read-only: a grade write for one is `409` (`404` when the user has nothing in the course), "publish
   all" skips them, and a listed publish names them as `not a current student`. Nothing is deleted
   when a student leaves, is disabled, is promoted or is excluded.
3. **Own grades.** `GET /courses/{id}/my-grades` also answers a user who is not on the roster, if they
   have published grades in the course; it is the only part of the course they can still read. A
   user without any gets the usual `404`.
4. **Role changes.** Changing a user from `teacher` or `admin` to `student` is refused with `409`
   while the user owns a course. Otherwise the same transaction deletes the user's `teacher`
   memberships and audits each one (`course.member.remove`, reason `role_change`). Group memberships
   stay (ADR-0032 decision 3) and do nothing while the user is not a student. Creating a course, adding
   a member with a role and transferring ownership lock the user's row before they read the role, so a
   concurrent demotion cannot slip a teacher in.
5. **Defence in depth.** `policy::authorize` ignores a `teacher` course role, and ownership, for an
   actor whose institution role is `student`. A state the rules above prevent is therefore also
   harmless if it ever occurs.
6. **Ownership transfer.** `PUT /courses/{id}/owner {user_id}` (`TransferOwnership`: the owner or an
   admin). The new owner is an active institution `teacher` or `admin`; a teacher becomes a course
   teacher in the same transaction, an admin owns without a membership. The previous owner stays a
   course teacher. The change is audited whoever makes it (`course.owner.transfer`).
7. **Rollover.** `PUT /groups/{id}/members {user_ids}` replaces the members of a group with exactly
   those users, and `POST /groups/{id}/members/remove {user_ids}` removes several. Both are all-or-
   nothing, lock the group row first, remove the subgroup memberships with the group membership and
   are audited. ARCHITECTURE.md describes the procedure: replace the members, rename the group.
   The previous year's students become former students of every linked course, with their results.
8. **Module dependency.** `identity` calls `teaching` (the owner check and the membership cleanup of
   decision 4) while `teaching` calls `identity` (the user lookups): a call cycle inside one crate,
   like `content` and `gradebook` in ADR-0030. It is recorded here, not hidden.

## Consequences
- A promoted or disabled person disappears from the current roster of every course at once, and their
  history stays readable to staff.
- The member list (`GET /courses/{id}/members`) no longer shows them; the gradebook still does, as
  former entries.
- A teacher who was added as a direct student and is then, say, demoted from admin is still on the
  roster. The rule for direct memberships is deliberately the ADR-0004 one.
- A teacher can still read the names of any group's members by linking the group to a course (ADR-0033
  decision 7). This ADR does not change who may link.
