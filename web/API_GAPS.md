# API gaps

What the screens need that `web/openapi.json` does not give yet. The backend was not touched: each gap has a typed hook in `src/lib/api/hooks/`, hand-written types in `src/lib/api/gaps.ts`, and an MSW handler in `src/mocks/handlers/` (run with `pnpm dev:mock`).
Delete the type, the hook branch and the handler when the server closes a gap.

`web/openapi.json` was generated from the server crate (`/api/v1/openapi.json`) at the commit this was built on. `pnpm gen:api` rebuilds `src/lib/api/schema.d.ts` from it.

Spec endpoints the UI uses as they are: auth, `/me`, `/me/sessions`, `/me/password`, `/me/courses`, courses and members, group links and options, the content tree, modules and items (create, patch, publish, reorder, delete), submissions and drafts, grades (save, publish), the gradebook, `my-grades`, admin users, groups, subgroups, terms, `/editor/profiles`.

## Institution
- **Screens:** navbar and sign-in (name), Course settings (grading scale), Admin, Institution.
- **Missing:** an endpoint for the institution name, short name, accent, default language, time zone and grading scale. An admin endpoint to read the server-side settings (failed logins, session lifetime, largest document) and to save the editable ones. Today they live in `learnly.toml` only.
- **Mocked:** `GET /institution` (any signed-in user, also before sign-in), `GET` and `PATCH /admin/institution`.

## Sign-in extras
- **Screens:** Sign in.
- **Missing:** self-registration ("Create an account"), Google sign-in, password reset by email. ADR-0017 plans all three; the server has none yet.
- **Mocked:** nothing. The buttons show a message that the feature is not available on this server yet.

## Course info
- **Screens:** the header of every course screen, Courses, Course settings.
- **Missing on `Course`:** `code`, `description`, the term's name, the owner's name and email, student, former-student and assistant counts, the course total and number of graded items, `has_grades`, and the time of the last settings save.
- **Mocked:** `GET /courses/{id}/info`, `PATCH /courses/{id}/info` (`code`, `description`). `PATCH /courses/{id}` is the real one for title and term.

## Course cards
- **Screens:** Courses (student and teacher), Home (My courses).
- **Missing:** one call that returns, per course of the caller: the course, code, term name and whether the term ended, owner name, the caller's role (owner, teacher, assistant, student), student and assistant counts; for a student the progress (submitted, total, graded) and the next deadline; for staff a to-do list (submissions to grade, draft grades not published, unpublished lectures, next deadline).
- **Mocked:** `GET /me/course-cards`.

## Templates
- **Screens:** Teacher Courses (Templates), Course settings (Reuse next term).
- **Missing:** counts of a template's content, creating a course from a template, making a template from a course. `Course.kind = template` exists, but nothing copies content (ADR-0005).
- **Mocked:** `GET /me/templates`, `POST /templates/{id}/courses`, `POST /courses/{id}/template`.

## Archive and delete
- **Screens:** Course settings (Archive or delete).
- **Missing:** `CourseStatus` has `archived`, but `UpdateCourseRequest` cannot set it, and there is no way to restore a course or to delete one.
- **Mocked:** `POST /courses/{id}/archive`, `POST /courses/{id}/restore`, `DELETE /courses/{id}` (409 when the course has grades, as ADR-0021 and ADR-0030 describe for assignments).

## My work (assignments with my own progress)
- **Screens:** Home (Due soon, Continue, New grades, month, Next 7 days), Calendar, Grades.
- **Missing:** one call for every assignment of the caller's courses with the caller's status (not started, draft, submitted, graded, missed, in class), score, publish time and draft time. With the spec alone this is a content tree, a submission and a grades call per course and per assignment.
- **Mocked:** `GET /me/work`. Only published scores and feedback are in it (root AGENTS.md section 6).

## Class date of in-class work
- **Screens:** Course content, Done in class, Calendar, Course content (teacher).
- **Missing:** the date a `no_submission` assignment takes place. The server keeps `due_at` `null` for that type, so the design's "In class on Thursday 29 October" has no field.
- **Mocked:** `class_on` in `GET /courses/{id}/item-meta` and in `PATCH /items/{id}`.

## Item meta
- **Screens:** Course content, the outline of every course screen, Course content (teacher).
- **Missing:** read time of a lecture, the class date, for a student the status, score, draft time, publish time and attempts used per assignment, for staff the "edited" flag and per-assignment counts (submitted, to grade, draft grades).
- **Mocked:** `GET /courses/{id}/item-meta`.

## Unpublished changes
- **Screens:** Lecture editor, Course content (the "Edited" label), Teacher home (Unpublished content).
- **Missing:** a published version next to the edited one. ADR-0010 says edits to published content are visible at once and there are no draft revisions in v1, so this contradicts it.
- **Mocked:** `GET /items/{id}/revision`, `POST /items/{id}/revision/publish`. A student keeps seeing the previous body and title until the changes are published.

## People of a course
- **Screens:** Course, People (student and teacher).
- **Missing:** names, emails and group labels. `Membership` carries ids only and teachers cannot read other users. Students must not receive other people's emails. A way to find someone to add: teachers cannot list users, only admins can.
- **Mocked:** `GET /courses/{id}/people`, `GET /courses/{id}/candidates?q=`.

## Teaching overview
- **Screens:** Teacher home, Grading, Calendar (teacher).
- **Missing:** the cross-course queue of submissions waiting for a grade, draft grades not published per assignment (with "scored automatically"), the next deadlines with submitted and total counts, unpublished content.
- **Mocked:** `GET /me/teaching`.

## Quiz and form assignment types
- **Screens:** Quiz, Quiz result, Form, Quiz builder, Form builder, Form responses.
- **Missing:** the types `quiz` and `form` (the server knows `text` and `no_submission`). Their `config`, the shape of a submission, automatic scoring, a result per question, and the responses of a form.
  - `config` for a quiz: `{questions: [{id, type, prompt, points, options, correct, answer, tolerance, partial_credit, shuffle, grading_notes}], shuffle_questions, show_correct}`. A student never receives `correct`, `answer`, `tolerance` or `grading_notes` before grades are published (ADR-0023).
  - `config` for a form: `{note, fields: [{id, type, label, help, required, options: [{id, text, taken_by}], rules, has_answers}]}`.
  - A submission's `content` is `{answers: {id: text | [ids] | block document}}`.
  - A question's prompt is a plain string here; ADR-0026 wants a block document.
  - A form is "not graded", but `max_points` must be above 0 (ADR-0021). The mock stores `"0.00"`.
  - "Remind who hasn't submitted" has no endpoint; the button only shows a message. The CSV download is built in the browser.
- **Mocked:** `GET /assignments/{id}/result`, `GET /assignments/{id}/responses`, `config` in `/items/{id}`, auto-scoring on submit.

## Grade history
- **Screens:** Gradebook (drawer).
- **Missing:** the recorded changes of one grade ("Draft set to 22.50, today 14:02").
- **Mocked:** `GET /assignments/{id}/grades/{student_id}/history`.

## Gradebook cells
- **Screens:** Gradebook.
- **Missing:** a flag for "submitted, waiting for a grade" in the cell. The UI asks `GET /assignments/{id}/submissions` for each column to know it.
- **Mocked:** nothing; the spec endpoints are used.

## Admin: people
- **Screens:** Admin, People.
- **Missing:** counts per role for the filter chips, a person's last sign-in, groups and courses, an import of people from CSV (the CLI has `import-users` planned).
- **Mocked:** `GET /admin/users/summary`, `GET /admin/users/{id}/facts`. The import creates the people one by one with `POST /admin/users`.

## Admin: groups
- **Screens:** Admin, Groups.
- **Missing:** group members with email and join date, student and course counts, the courses a group is linked to with their owners.
- **Mocked:** `GET /admin/groups/{id}/overview`.

## Admin: terms and courses
- **Screens:** Admin, Terms and courses.
- **Missing:** the number of courses per term and whether a term is current or read only; a list of every course with owner, student count and status.
- **Mocked:** `GET /admin/term-stats`, `GET /admin/courses`. Changing an owner is the real `PUT /courses/{id}/owner`.

## Admin: audit log
- **Screens:** Admin, Audit log.
- **Missing:** a read endpoint for `events.audit_log` with filters (person, kind of change, period).
- **Mocked:** `GET /admin/audit?q=&kind=&days=`. Changes made through the mock add entries.

## Admin: accent colour and logo
- **Screens:** Admin, Institution.
- **Missing:** storing an accent colour and a logo; the design does not say how a chosen accent gets a dark-theme match.
- **Mocked:** the choice is saved in the mock institution and shown in the preview only. The site keeps its tested accent. Upload is disabled.

## Search
- **Screens:** navbar.
- **Missing:** a search over courses and materials. The field filters the course list by title and code in the browser.
- **Mocked:** nothing.

## Not API gaps, kept in the browser
Theme, language, and the layout of the home widgets are saved in `localStorage`, as the design says. No translation catalogue exists: the language setting is stored but not applied.
