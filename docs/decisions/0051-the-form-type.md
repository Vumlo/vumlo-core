# 0051. The `form` assignment type

Status: Accepted
Supplements: ADR-0021, ADR-0023, ADR-0035, ADR-0050

## Context
ADR-0023 defines `form` as fields the teacher sets and the student fills in (team name, repository link,
description), graded by hand. The web app builds forms with seven field types, lets a field offer
options that only one student may take (a topic, a variant), and shows the teacher a table of the
answers with the students who have not answered yet.

## Options
- **Who sees who took an option.** (a) Every student sees the name, as the first web mock did: it
  tells a student who else chose what. (b) **A student sees that an option is taken; staff see by
  whom.**
- **Where the "taken" and "has answers" facts live.** (a) Stored in the `config` by the server: the
  teacher's `PATCH` would race with the submits. (b) **Computed when the config is read** and never
  stored.
- **Exclusive options.** (a) Check, then insert: two students can both win. (b) **A transaction
  advisory lock per assignment around the check and the submit.**

## Decision
1. **Config.** The `config` of a `form` is `{"v": 1, "note": string, "fields": [...]}`: `note` at most
   1000 characters (may be empty), 1 to 50 fields. Unknown fields are `422`. `max_points` is above 0
   like every assignment (ADR-0021); the teacher grades by hand (ADR-0023). Nothing about the points
   is tied to the fields.
2. **A field** has an `id` (the rules of ADR-0050 point 2), `type`, `label` (1 to 200 characters),
   `help` (at most 500, may be empty), `required` (bool) and `rules`. By type:
   - `short`: `rules.max_length` (1 to 500, default 200). The answer is a string.
   - `long`: `rules.max_length` (1 to 10000, default 5000). The answer is a string.
   - `choice`: `options` (2 to 20 of `{id, text}`, `text` 1 to 200 characters, ids unique in the
     field) and `rules.exclusive` (bool, default false), `rules.multiple` (bool, default false). The
     answer is a list of option ids, one unless `multiple`.
   - `people`: `rules.min` and `rules.max` (whole numbers, `0 <= min <= max <= 10`, default 0 and 5).
     The answer is a list of ids of current students of the course, no repeats, the submitter not
     counted in the limits and not listed.
   - `link`: the answer is an `http` or `https` URL of at most 2048 characters.
   - `number`: `rules.min` and `rules.max` (optional decimals). The answer is a decimal string.
   - `date`: the answer is a date `YYYY-MM-DD`.
   A rule that does not belong to the type, and a field of another type's shape, are `422`. Unknown
   rules are `422`.
3. **Answers.** The content of a draft and of an attempt is `{"answers": {field id: value}}`, with the
   values above. An entry that is not a field, or of the wrong shape or beyond a rule, is `422` on
   `content.answers.<id>`. Saving a draft checks shapes and rules but not `required`, because a draft
   is unfinished. A submit also requires every `required` field to have a non-empty answer (`422` on
   `content.answers.<id>`, "is required") and that the ids of a `people` field are current students of
   the course. A submit with no answer at all is `422` (ADR-0035 point 3).
4. **Exclusive options.** A `choice` field with `rules.exclusive` gives each option to one student: the
   students whose latest attempt holds it. A submit takes `pg_advisory_xact_lock` on the assignment,
   reads the latest attempts of the other students, and is `409` if one of the chosen options is held.
   A student who submits again keeps their own options. The draft is not checked: an option can be taken
   while a student is still typing.
5. **Facts on read.** `GET /items/{id}` adds to the `config` it returns, and never stores them:
   `has_answers` (bool) on each field for staff, true when any submitted attempt has an entry for it
   (the builder uses it to warn); and on each option of an exclusive field `taken` (bool, held by
   another student, for everyone but the holder themselves) and, for staff only, `taken_by` (the
   holder's name and surname). A `PATCH` that sends these back has them ignored: they are dropped before
   the config is checked and are not stored.
6. **Structure is frozen** after the first submitted attempt (ADR-0023): the ids, types, `required`,
   `rules` and option ids of fields and the order of fields cannot change (`409`). `note`, `label`,
   `help` and option `text` may.
7. **Responses.** `GET /assignments/{id}/responses` (staff: `ViewSubmissions`) answers
   `{total_students, missing: [{user_id, name, surname}], rows: [{student_id, name, surname, attempt,
   submitted_at, is_late, answers}]}`: one row for each current student who has submitted, with the
   latest attempt's `answers` as stored, and `missing` the current students with no attempt. It is not
   paged: a course roster is bounded (ADR-0033). The client builds its columns from the config.
8. A `form` has no auto grade and no result endpoint (`404` for `/result`).

## Consequences
- The exclusive check serializes the submits of one form only; other assignments are unaffected.
- A student who left the roster keeps their claims (their latest attempt still holds the option) until a
  teacher frees it by a new design; releasing a former student's options is not decided here.
- The people ids are checked when the form is submitted only; a student who leaves later stays named.
- Responses for a very large roster would need paging: not decided here.
- "Remind who has not submitted" is not built (it needs notifications).
