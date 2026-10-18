# 0050. The `quiz` assignment type

Status: Accepted
Supplements: ADR-0015, ADR-0023, ADR-0026, ADR-0028, ADR-0035, ADR-0049
Amends: ADR-0026 (the question object gets more types and points), ADR-0035 (the submission content is no longer always a block document)

## Context
ADR-0023 names `quiz` as an assignment type that is graded automatically as a draft grade, and leaves
its JSON to this ADR. ADR-0026 fixed the question object (`single_choice`, `multiple_choice`, prompts
and options as block documents) and promised that the quiz ADR adds the other types, points and
attempts. The web app builds quizzes with five question types and shows a student the result of a
published quiz.

Until now the content of a submission is always one block document (ADR-0035), the trait method
`validate_submission` returns a document, and the whole `config` goes to every reader (ADR-0035,
known limits). A quiz needs all three to change.

## Options
- **What a submission holds.** (a) One block document with the answers inside: the document model has
  no answer block. (b) **`{"answers": {question id: value}}`**: it is what the question ids are for
  (ADR-0023: "submissions refer to them by id").
- **How a question is written.** (a) Plain strings for prompt and options, as the first web mock did.
  (b) **Block documents (ADR-0026)**: formulas and code in a prompt are the common case for this
  institution, and ADR-0026 already decided it.
- **Who writes the grade.** (a) `assessment` writes `gradebook` tables: forbidden (AGENTS.md section 4).
  (b) **`assessment` calls one public function of `gradebook`** with the connection, as `gradebook`
  calls `events::emit`.
- **A quiz with a free-text question.** (a) Grade nothing: the teacher re-enters everything. (b) Write
  a draft grade without the free answer: wrong, it looks complete. (c) **Score what can be scored and
  keep the per-question scores, but write a draft grade only when every question was scored.**
- **The answer key for a student.** (a) A second "check" endpoint for each question (ADR-0026): it
  lets a student find the answer by trying, which ADR-0026 itself says a graded quiz must not allow.
  (b) **Never send it**; the student sees correct answers only after their grade is published, and
  only if the teacher turned `show_correct` on.

## Decision
1. **Config.** The `config` of a `quiz` is `{"v": 1, "questions": [...], "shuffle_questions": bool,
   "show_correct": bool}` with 1 to 100 questions. Unknown fields are `422` on `config`. `max_points`
   of the assignment must equal the sum of the `points` of its questions (`422` on `max_points` if a
   change of the points or of the questions would break that). The sum is judged on the state a write
   would leave, after the assignment row is locked.
2. **A question** has an `id` (1 to 64 characters of `A-Za-z0-9_-`, unique in the quiz, stable),
   `type`, `points` (above 0, at most two decimals, a decimal string like `max_points`), `prompt` (a
   block document, profile `question`) and optionally `explanation` (same profile) and
   `grading_notes` (plain text for staff, at most 2000 characters). By type:
   - `single_choice` and `multiple_choice`: `options` (2 to 10 of `{id, content}`, ids unique in the
     question, `content` a block document of profile `option`), `correct` (the ids of the correct
     options: exactly one for `single_choice`, at least one for `multiple_choice`), `shuffle_options`
     (bool, default false). `multiple_choice` also has `partial_credit` (bool, default false).
   - `number`: `answer` and `tolerance` (decimal strings; `tolerance` is at least 0, default `"0"`).
   - `short_text`: `accepted`, 1 to 20 strings of 1 to 200 characters after trimming.
   - `long_text`: nothing more. It is graded by hand.
   A field that does not belong to the type is `422`.
3. **Answers.** A submission's content (draft and attempt) is `{"answers": {question id: value}}`.
   A value is, by type: a list of option ids (`single_choice`: at most one; `multiple_choice`: no
   repeats), a decimal string (`number`; a comma is taken as the decimal point), a string of at most
   500 characters (`short_text`), a block document of profile `student` (`long_text`). A question
   without an entry, or with an empty value, is unanswered. An id that is not a question, an option
   that is not of the question, or a value of the wrong shape is `422` on `content.answers.<id>`. A
   draft is checked the same way, so a client learns of a mistake while it autosaves. A submit with no
   answered question is `422` ("empty", ADR-0035 point 3). The trait method `validate_submission`
   returns the checked content and whether it is blank, for every type.
4. **Scoring.** At submit, in the submit's transaction, the server scores every question it can:
   `single_choice` all or nothing; `multiple_choice` all or nothing, or with `partial_credit`
   `points * max(0, good - bad) / correct_count` rounded to two decimals, where `good` is the chosen
   correct options and `bad` the chosen wrong ones; `number` full points when `|given - answer| <=
   tolerance`; `short_text` full points when the trimmed answer equals one of `accepted`, ignoring
   case; an unanswered question scores 0. `long_text` is not scored. The scores are stored with the
   attempt as `assessment.submissions.auto_scores` (a JSON object of question id to decimal string,
   without the `long_text` questions), set by the same statement that turns the draft into the
   attempt, so an attempt never changes (ADR-0035 point 1).
5. **The draft grade.** When the quiz has no `long_text` question, the submit also calls
   `gradebook::save_auto_grade` with the sum. It replaces the draft score of the student's cell (a
   later attempt replaces the score of an earlier one) and leaves the draft feedback and the published
   value alone, and appends a history entry with the action `auto_score` and no person. The migration
   makes `gradebook.grades.draft_updated_by` and `gradebook.grade_history.actor_id` nullable, with a
   check that a history row has no person exactly when its action is `auto_score`. Nothing is
   published: a teacher still publishes (ADR-0015, ADR-0028). With a `long_text` question no cell is
   written and the teacher grades by hand with the per-question scores in front of them. The save
   emits no event (ADR-0007).
6. **The student's view of `config`.** `GET /items/{id}` for a student returns the question list
   without `correct`, `answer`, `tolerance`, `accepted`, `explanation`, `partial_credit` and
   `grading_notes`. The trait gets `student_config`; the other types return the whole `config`. Staff
   get everything. AGENTS.md section 6 gets the invariant, and a test.
7. **Structure is frozen** after the first submitted attempt (ADR-0023): a change of the question
   list's ids, types, option ids, `correct`, `answer`, `tolerance`, `accepted`, `points`,
   `partial_credit` or the order of questions is `409`. Wording (`prompt`, `explanation`, option
   `content`, `grading_notes`), `shuffle_options`, `shuffle_questions` and `show_correct` may change.
   The trait gets `structure`, the part of the config that is frozen; the check runs under the
   assignment row lock, like every settings change (ADR-0035 point 12).
8. **The result.** `GET /assignments/{id}/result` answers the caller, a student, only when their grade
   for the assignment is published; before that, and for a quiz the caller has not attempted, it is
   `404`. It answers `{score, max_points, published_at, feedback, attempt, questions: [{id, got,
   points, answer, correct}]}` for the latest attempt. `got` is the stored per-question score (`null`
   for a `long_text` question: staff grade that part as a whole, so the question carries no score of
   its own). `answer` is what the student gave. `correct` (the correct option ids, the answer, or the
   accepted strings) and `explanation` are present only when `show_correct` is on. Only published values and the
   student's own attempt are read (AGENTS.md section 6). A published score a teacher changed by hand
   shows as the published `score`; `got` stays what the server computed.
9. Staff see the per-question scores in the attempts they already read: `auto_scores` is an additive
   field of the attempt DTO, `null` for the other types.

## Consequences
- Every submission path stops assuming a document: the trait returns checked content and a blank flag.
- A student can retry as many times as `max_attempts` allows (ADR-0035) and each attempt rewrites the
  draft score. Which attempt counts is therefore always the latest; a "best of" rule is a later ADR.
- A teacher who typed a draft score by hand loses it to the next attempt's auto score. The history
  keeps both.
- A quiz with a `long_text` question never gets a draft grade, even if a teacher had one. The score of
  the other questions is on the attempt for the teacher to add up.
- The question ids are the only link between a question and the answers; wording changes keep them.
- Not decided here: time limits, question pools, random variants, group quizzes, a "best attempt" rule,
  releasing the result before the grades are published.
