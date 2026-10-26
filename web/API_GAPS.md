# API gaps

What the screens need that `web/openapi.json` does not give yet. The backend was not touched: each gap has a typed hook in `src/lib/api/hooks/`, hand-written types in `src/lib/api/gaps.ts`, and an MSW handler in `src/mocks/handlers/` (run with `pnpm dev:mock`).
Delete the type, the hook branch and the handler when the server closes a gap.

`web/openapi.json` was generated from the server crate (`/api/v1/openapi.json`) at the commit this was built on. `pnpm gen:api` rebuilds `src/lib/api/schema.d.ts` from it.

Spec endpoints the UI uses as they are: `GET /institution`, `GET /admin/institution` (read-only; the settings live in `vumlo.toml`, ADR-0044), auth, `/me`, `/me/sessions`, `/me/password`, `/me/courses`, courses and members, group links and options, the content tree, modules and items (create, patch, publish, reorder, delete), submissions and drafts, grades (save, publish), the gradebook, `my-grades`, admin users, groups, subgroups, terms, `/editor/profiles`.

## Sign-in extras
- **Screens:** Sign in, Create an account, Confirm your email, Reset your password, Choose a new password, Google callback.
- **Server:** done (ADR-0045 to ADR-0047). Registration and reset answer `202` with no body for every address; an account has no password until its address is verified and the owner chooses one on the page behind the link (ADR-0067), so a login before that is `401`; Google needs `[auth.google]` and the cookie `vumlo_oauth`, which the browser keeps between start and callback.
- **Still mocked, on purpose:** `pnpm dev:mock` and the Playwright suite (`e2e/`) run against the in-browser mock, and they cover these screens. Delete the handlers in `src/mocks/handlers/auth.ts`, `MockMailbox` and `useMockOutbox` when the suite runs against the real server. The mock differs from the server in its rules (it wants 10 characters, the server 12 and a strength score) and in the `hint` the mock puts in the callback URL.
- **What the mock does:**
  - `POST /auth/register` (`name`, `surname`, `email`; 202; 422 for an address outside `northfield.edu`), `POST /auth/verify-email` (`token`, `password`; 410 when spent, 422 without a password).
  - `POST /auth/password-reset` (`email`; 202), `POST /auth/password-reset/confirm` (`token`, `new_password`; 410 when spent).
  - `POST /auth/google/start` (`login_hint`) answers `{authorization_url}`; the page opens it. `POST /auth/google/callback` (`code`, `state`) opens the session. The mock skips Google's consent screen and signs in as the account named by `login_hint`.
  - `GET /mock/outbox`: only in the mock. The pages show a "Mock mailbox" box with the links a real mail would carry. The mock keeps its data in the page, so a full reload forgets pending sign-ups and reset links; follow the links without reloading.
- **Not mocked:** the "must change password" rule of ADR-0017 (a temporary password may only call change password, logout and `GET /me`). The page does not rely on it; it sends such a session to `/change-password` itself.

## Search
- **Server:** done (ADR-0052). `GET /search?q=` (2 to 100 characters) over the caller's courses, titles first, at most 50, students on the published copy. An administrator who is on no roster finds nothing.
- **Still mocked, on purpose:** the same endpoint in `src/mocks/handlers/work.ts`.

## New course
- **Screens:** New course.
- **Missing:** `POST /courses` takes only `title` and `term_id`. The page also sets `code` and `description` with `PATCH /courses/{id}/info`, and a copy from a template can take a `term_id` now (`POST /templates/{id}/courses`). The page makes these as follow-up calls, and still opens the settings page if one of them fails.

## Language
- **Screens:** every screen, Admin Institution (default language), Settings (this device).
- **Missing:** nothing on the server. The institution default comes from the mocked `GET /institution`.
- **Note:** ADR-0043 describes the catalogue. Every screen is translated. Text written by the server is not: the to-do lines on course cards (`GET /me/course-cards`), the notes in `GET /me/teaching`, grade history and audit lines, the facts in `GET /admin/users/{id}/facts`. They should become structured data (a kind and counts), so the client can word them.

## Course screens (info, templates, archive and delete, people)
- **Server:** done (ADR-0048). `GET`/`PATCH /courses/{id}/info`, `POST /courses/{id}/archive` and `/restore`, `DELETE /courses/{id}`, `POST /courses/{id}/template`, `POST /templates/{id}/courses`, `GET /me/templates`, `GET /courses/{id}/people`, `GET /courses/{id}/candidates?q=` are in `openapi.json`. An archived course is read-only (`409`) and still readable by its students.
- **Differences the screens absorb:** the owner comes as `owner_name` and `owner_surname` (no "Dr."), `settings_saved_at` is `updated_at`, a linked group is `group_name` and `subgroup_name`, and a former student has no `left_on` date.
- **Still mocked, on purpose:** the same endpoints in `src/mocks/handlers/core.ts`, with the server's shapes, for `pnpm dev:mock` and the Playwright suite. The mock does not make an archived course read-only.

## Course cards
- **Server:** done (ADR-0052). `GET /me/course-cards`: the owner is `owner_name` and `owner_surname`; a teacher's to-do is a list of tagged entries (`to_grade`, `draft_grades`, `unpublished_items`, `next_deadline`) with counts, worded by `todoText` in `src/lib/course.ts`.
- **Still mocked, on purpose:** the same endpoint in `src/mocks/handlers/core.ts`.

## My work (assignments with my own progress)
- **Server:** done (ADR-0052). `GET /me/work` lists the assignments students can open in the caller's courses with the status of ADR-0049, the published score and times. The class day is a date; the hook turns it into noon of that day.
- **Still mocked, on purpose:** the same endpoint in `src/mocks/handlers/work.ts`. Only published scores and feedback are in it (root AGENTS.md section 6).

## Class date of in-class work
- **Server:** done (ADR-0049). `class_on` is a day (`YYYY-MM-DD`) on `no_submission` assignments, in `PATCH /items/{id}` and `GET /courses/{id}/item-meta`; the hook turns it into noon of that day in the institution's zone.
- **Still mocked, on purpose:** the same fields in `src/mocks/handlers/core.ts`.

## Item meta
- **Server:** done (ADR-0049). `GET /courses/{id}/item-meta` gives read time, class day, a student's status, score, draft and publish times and attempts used, and for staff the "edited" flag and per-assignment counts.
- **Still mocked, on purpose:** `src/mocks/handlers/core.ts`, in the wire shape.

## Unpublished changes
- **Server:** done (ADR-0049, supersedes the last decision of ADR-0010). `GET /items/{id}/revision` and `POST /items/{id}/revision/publish`; a student keeps seeing the published title and body until the changes are published.
- **Still mocked, on purpose:** the same endpoints for `pnpm dev:mock`.

## Teaching overview
- **Server:** done (ADR-0052). `GET /me/teaching` returns `to_grade`, `unpublished`, `deadlines` (`not_open`) and `unpublished_content` (`reason: draft | edits`) over the caller's courses that are not archived; the to-grade queue is cut at 200.
- **Still mocked, on purpose:** the same endpoint in `src/mocks/handlers/work.ts`.

## Quiz and form assignment types
- **Server:** done (ADR-0050, ADR-0051). `quiz` and `form` are assignment types; `GET /assignments/{id}/result` and `GET /assignments/{id}/responses` are in `openapi.json`. A quiz is scored when it is submitted and its score becomes the draft grade (not when it has a free-text question); a student never receives the answer key.
- **Differences the screens absorb** (`src/lib/api/sheetWire.ts`, the one place that knows both shapes): the screens keep a prompt and an option as plain text, the server stores block documents; the screens' `single`, `multi`, `short`, `long` are `single_choice`, `multiple_choice`, `short_text`, `long_text`; a single-choice answer is a list with one id; a short-text question has one accepted answer in the screens (the server takes a list); a form's team size `"2-4"` is `rules.min` and `rules.max`; the responses table has fixed columns (team, variant, members, repository), filled from the first short, choice, people and link fields.
- **Differences in behaviour:** a form needs `max_points` above 0 like any assignment; `taken_by` reaches staff only, a student gets `taken`; the questions and fields are frozen after the first submitted attempt (`409`).
- **Not built:** "remind who hasn't submitted" (the button only shows a message); the CSV download is built in the browser.
- **Still mocked, on purpose:** the same endpoints and fields in `src/mocks/handlers`, in the server's shapes.

## Grade history
- **Server:** done (ADR-0049). `GET /assignments/{id}/grades/{student_id}/history` returns structured entries (`action`, `score`, `feedback`, `actor`), newest first, cursor-paginated. The drawer words them with the `gb.h*` messages.
- **Still mocked, on purpose:** the same endpoint in `src/mocks/handlers/work.ts`.

## Gradebook cells
- **Screens:** Gradebook.
- **Missing:** a flag for "submitted, waiting for a grade" in the cell. The UI asks `GET /assignments/{id}/submissions` for each column to know it.
- **Mocked:** nothing; the spec endpoints are used.

## Admin: people
- **Server:** done (ADR-0053). `GET /admin/users/summary` and `GET /admin/users/{id}/facts` (groups, owned and taught courses, courses studied, last sign-in); the details list and the role note are worded by the screen.
- **Still missing:** an import of people from CSV (the import creates the people one by one with `POST /admin/users`).
- **Still mocked, on purpose:** the same endpoints in `src/mocks/handlers/admin.ts`.

## Admin: groups
- **Server:** done (ADR-0053). `GET /admin/groups/{id}/overview`: members with email and the day they joined (`added_at` is `null` for those who were in before it was recorded), counts, and the courses that use the group.
- **Still mocked, on purpose:** the same endpoint in `src/mocks/handlers/admin.ts`.

## Admin: terms and courses
- **Server:** done (ADR-0053). `GET /admin/term-stats` (courses, `current`, `read_only` for each term) and `GET /admin/courses` (paged). Changing an owner is the real `PUT /courses/{id}/owner`.
- **Still mocked, on purpose:** the same endpoints in `src/mocks/handlers/admin.ts`.

## Admin: audit log
- **Server:** done (ADR-0053). `GET /admin/audit?q=&kind=&days=`: entries with the action, the actor, what the target is called and the metadata; `src/lib/audit.ts` words each action. The kinds are `accounts`, `owners`, `teachers`, `groups`, `courses`, `content`, `grades`, `terms`, `other`. Names are those of today, and details such as the previous owner are not worded.
- **Still mocked, on purpose:** the same endpoint in `src/mocks/handlers/admin.ts`; changes made through the mock add entries.

## Admin: logo
- **Screens:** Admin, Institution.
- **Missing:** storing and serving a logo; it needs the `files` module. The accent is set in `vumlo.toml` and shown in the preview only; the site keeps its tested accent because the design does not say how a chosen accent gets a dark-theme match.

## Not API gaps, kept in the browser
Theme, language, and the layout of the home widgets are saved in `localStorage`, as the design says.
