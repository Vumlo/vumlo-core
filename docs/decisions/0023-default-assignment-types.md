# 0023. Default assignment types

Status: Accepted
Amends: ADR-0008, ADR-0015
Supplements: ADR-0021

## Context
ADR-0008 made assignment types extensible and named the v1 built-ins: `file`, `text`, `form`, `no_submission`, where `form` meant questions that can be auto-graded. Two different needs hide in that word. One is a set of fields the teacher defines and the student fills in (team name, repository link, description). The other is a test with correct answers. They differ in what a student submits, how it is graded and what a student may see, so they become two types.

## Options
- (a) Separate entities with their own tables (quizzes, forms, uploads): each needs its own tree rows, publishing, gradebook link and deletion rules, and the gradebook of ADR-0009 ("every column is an assignment") would have to learn each of them.
- (b) **One entity, the assignment.** The type is its `type`, its `config` and the shape of `submissions.content`. A quiz builder is a client of the same API that edits `config`.
- (c) One generic `form` type for everything: a quiz is not a form with a flag, because it needs answer keys hidden from students, auto-grading and attempt rules.

## Decision
- The built-in types for v1 are `text`, `no_submission`, `file`, `form` and `quiz`. All of them are assignments (ADR-0021): the same table, the same place in the course tree, the same publishing, `max_points` and gradebook column.
- What each type is. The exact JSON of each is decided by its own ADR in the task that builds the type, except where another ADR is named.

  | Type | The student submits | Graded |
  |---|---|---|
  | `text` | a block document (ADR-0025, profile `student`) | by hand |
  | `no_submission` | nothing (oral answers, class work, paper tests) | by hand |
  | `file` | one or more uploaded files | by hand |
  | `form` | answers to fields the teacher defines (for example team name, repository link, description) | by hand |
  | `quiz` | answers to questions the teacher builds, in the question model of ADR-0026 plus points and attempts | automatically, as a draft grade (ADR-0015) |

- The description of every assignment is the item's `body`, a block document (ADR-0024, ADR-0025). The `config` of a `text` assignment will be able to narrow the editor features offered to the student (ADR-0025); that arrives with the submissions task.
- `form` fields and `quiz` questions live in `config`, each with a stable id. Submissions refer to them by id.
- Once a submitted attempt exists, the structure of a `form` or `quiz` (fields, questions, options, answer keys) cannot change: the write is refused with 409. Which wording edits stay allowed is decided in each type's ADR.
- A student never receives an answer key. The trait gets a student view of `config` when the first type needs it (`quiz`). The default view is the whole `config`; `quiz` removes correct answers and explanations until the rules of its ADR allow them. AGENTS.md §6 gets a matching invariant and a test then.
- The `AssignmentType` trait grows task by task, only with what the task uses: now `validate_config`; the submissions task adds `validate_submission`; `form` and `quiz` add the student view; `quiz` adds `auto_grade`, which writes a draft grade through `gradebook`.
- Submissions of every type are rows of `assessment.submissions` whose `content` the type shapes. A team name in a `form` does not make a team submission: submissions stay per student (ADR-0011), and group work needs its own ADR.
- Whether students see `no_submission` items in the course tree is decided with the gradebook.

## Consequences
- The word `form` changes meaning from ADR-0008. `assessment` auto-grades `quiz` assignments, not `form`; the phrase "auto-grading of `form` assignments" in ADR-0015 and the mention of `form` in ADR-0021's Consequences now mean `quiz`.
- The gradebook needs no per-type code: every type is a column.
- Adding a type is one enum variant, one implementation of the trait and one ADR for its JSON.
- The `form` and `quiz` builders are frontend features over `config`; the API has no quiz-specific entities.
- Order of work: `no_submission` before the gradebook; submissions together with `text`; `file` after the `files` module; `form`; `quiz` after submissions and the gradebook.
