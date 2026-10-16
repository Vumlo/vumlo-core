# 0026. Lessons and self-check questions

Status: Accepted
Supplements: ADR-0023, ADR-0024, ADR-0025

## Context
A lesson is a short presentation a teacher assembles from steps in any order: a text page, a self-check question, a link or video, a file. A quiz question and a self-check question are the same object, graded in one and not in the other.

## Options
Lesson: (a) an item containing child items: deeper tree, per-step publishing nobody asked for, more rows; (b) **one item whose body holds the ordered steps**; (c) one long text document with page breaks: it cannot hold structured questions.

Checking an answer: (a) send the answer key to the client: that breaks "a student never receives an answer key" (ADR-0023); (b) **a server-side check endpoint**.

## Decision
- A `lesson` item's body is `{"v": 1, "steps": [...]}` with 1 to 100 steps in order. Every step is an object with an `id`, unique in the lesson and stable, and a `type`:
  - `text`: `doc`, a block document (ADR-0025, profile `full`);
  - `question`: `question`, see below;
  - `link`: `url` and `embed`, with the rules of ADR-0024;
  - `file`: `file_id`, once the `files` module exists.
  A video is a `link` with `embed` or an uploaded `file`. A lesson is at most 2 MiB.
- The whole lesson is one item: it is published, scheduled and deleted as one, with no per-step visibility. Progress and completion are a later ADR; nothing a student does in a lesson is stored.
- The question object, shared with the `quiz` assignment type: `{"id", "type": "single_choice" | "multiple_choice", "prompt", "options": [{"id", "content"}], "correct": [option id], "explanation"?}`. `prompt` and `explanation` are block documents (profile `question`), each option's `content` a block document (profile `option`). A question has 2 to 10 options with unique ids; `single_choice` has exactly one correct option and `multiple_choice` at least one. More question types, points and grading come with the `quiz` ADR. A question in a lesson never has points.
- The student's view of a lesson contains no `correct` and no `explanation` in any step. Staff see everything. AGENTS.md §6 gets this invariant and a test when the lesson is built.
- Checking: `POST /items/{id}/steps/{step_id}/check` with `{"choices": [option ids]}` answers `{"correct": bool, "explanation"?: block document}`. It needs the right to view the lesson (`ViewContent`), writes nothing, is not audited, emits no event and does not limit repeats. It does not list the correct options. An invisible lesson and an unknown step are 404; choices that are not options of the question are 422 on `choices`.

## Consequences
- Because the check is ungraded and repeatable, a student can find the answer by trying. That is accepted for self-checks; the graded `quiz` must not reuse this endpoint.
- Editing a lesson saves the whole body; the 2 MiB limit bounds that.
- The structure of a lesson is not frozen, because nothing in it is graded.
- The `quiz` ADR extends the question types and adds points and attempts to the same object.
