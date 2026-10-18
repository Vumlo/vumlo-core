# 0052. Home read models and search

Status: Accepted
Supplements: ADR-0015, ADR-0028, ADR-0035, ADR-0048, ADR-0049

## Context
The web app's home screens need data that no single endpoint gives: the course cards (with a student's
progress or a teacher's to-do), a student's work across all their courses, a teacher's queue across
all theirs, and a search over courses and materials. Until now the screens computed this in the
browser from many calls, or from mocks that wrote English sentences (to-dos, notes).

## Options
- **Where the work is done.** (a) The client composes existing endpoints: one request per course, and
  a student's work needs every course tree. (b) **One read-model endpoint for each screen**, computed
  by SQL in the module that owns the screen's subject (ADR-0015 allows JOINs in read models).
- **Text.** (a) The server writes "3 submissions to grade". (b) **It sends the numbers and kinds; the
  client words them** (ADR-0043).
- **Paging.** The results are bounded by what one person teaches or studies, so they are not paged;
  the bound is named in the code. Search is capped at 50 and says so.

## Decision
1. **Who.** All four endpoints need a session and use `Action::ViewOwnCourses` (any role). What they
   return is scoped by membership in SQL: courses the caller is on the roster of or owns, regular
   courses only (never a template). A student sees a course only while it is published or archived
   (ADR-0048), and in it only the modules and items that are published and whose `publish_at` has
   passed (the rule of the course tree), with the **published** title and body (ADR-0049) and
   **published** grades only (AGENTS.md section 6).
2. **`GET /me/course-cards`** (`teaching`): one card per course: the course, `code`, `term_name`,
   `term_ended` (the term's last day is before today in the institution's time zone), the owner's name and
   surname, `my_role` (`owner`, `teacher`, `assistant`, `student`), `student_count`, `assistant_count`.
   A student's card has `progress` `{submitted, total, graded}` over the assignments that take
   submissions and `next` (the earliest upcoming deadline of work not yet submitted). A staff card has
   `todo`, a list of tagged entries: `to_grade {count}` (submitted work with no draft score),
   `draft_grades {count, auto_scored}` (draft grades not published; `auto_scored` when they belong to
   quizzes), `unpublished_items {count, titles}` (draft items inside published modules, at most three
   titles) and `next_deadline {assignment_id, title, due_at}`. Entries with a count of 0 are left out.
3. **`GET /me/work`** (`content`): the courses the caller studies in, and every assignment visible to
   them with the work status of ADR-0049 point 6, the published score, `published_at`, `draft_saved_at`,
   `due_at`, the `class_on` day and `max_points`.
4. **`GET /me/teaching`** (`gradebook`): over the courses the caller teaches, assists in or owns and that
   are not archived: `to_grade` (the latest attempt of each current student with no draft score, oldest
   first, at most 200), `unpublished` (assignments with draft grades not published: `draft_count`,
   `auto_scored`), `deadlines` (assignments due from now on, soonest first: how many of the current
   students submitted, `not_open` when the assignment is a draft) and `unpublished_content` (items not
   published in a published module, `reason: draft`, and published items with unpublished edits,
   `reason: edits`).
5. **`GET /search?q=`** (`content`): `q` is 2 to 100 characters after trimming (`422` otherwise). It
   matches, case-insensitively and as a substring with `%`, `_` and `\` escaped, a course's title, code and
   description, and an item's title and the text of its body. Staff get drafts and the working copy; a
   student gets the published copy. Hits are `{kind: course | lecture | assignment, id, title, course_id,
   course_title, course_code, snippet, due_at, status}`, a title match before a body match, at most 50, no
   cursor (`next_cursor: null`). The snippet is cut in code from the plain text of the body.
6. None of them is audited (reads) and none emits an event.

## Consequences
- A student's `/me/work` and the cards cost one query per kind, not per course.
- The body match is a scan with `ILIKE` over the JSON text of the bodies of the caller's courses. It is
  fine for the size of one institution's courses; a trigram or full-text index is the upgrade when it is
  slow (named in the code).
- The to-do and note texts of the old mocks are gone; the client words each tagged entry.
- An administrator who is on no roster sees no cards and no search hits; the administrator screens are
  separate (ADR-0053).
- Not decided here: notifications, a "reminders" feed, search over files and over the contents of
  quiz questions.
