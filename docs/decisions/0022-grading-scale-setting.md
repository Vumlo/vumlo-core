# 0022. The grading scale is an institution setting

Status: Accepted
Supplements: ADR-0002, ADR-0009, ADR-0021
Amends: ADR-0004

## Context
ARCHITECTURE.md planned an `org.grading_scales` table and a `courses.grading_scale_id` column. The real need is smaller. One installation serves one institution (ADR-0002), so one scale per installation is enough. A school grades every piece of work on a 1–12 scale. A university course adds points up to a total of 100, and may offer bonus points on top. Assignment points are `numeric(6,2)` (ADR-0021), and final grades are outside v1 (ADR-0009).

## Options
Where the scale lives: (a) a table of scales referenced from each course — nobody asked for per-course scales, and it costs a migration, seed data and a backfill; (b) **a setting in the config file**, defaulting from `institution.mode`; (c) hard-coded by mode — a school that wants 100 points cannot get it.

What the scale does in v1: (a) nothing, only reserved for final grades — a school could create a 37-point assignment, a university course could hold 400 regular points, and "the course grade out of 100" would mean nothing; (b) **it limits points**, so the stored data already fits the scale.

Bonus points at a university: (a) no distinction, the cap counts everything — a course cannot offer extra credit; (b) no cap at all — nothing stops a course from carrying 400 regular points; (c) **a flag `is_bonus` on the assignment; the cap counts regular points only**.

How the sum is kept under concurrent writes: (a) lock the course row through a `teaching` function — couples `content` to `teaching`'s locking and queues unrelated writers; (b) **a transaction advisory lock on the course id, taken by `content`** — touches no row of another module; (c) no lock — two concurrent creates can both pass and exceed the limit.

## Decision
- There is no `org.grading_scales` table and no `courses.grading_scale_id`.
- The scale is `institution.grading_scale = twelve | hundred` in `learnly.toml`. Until `institution.mode` exists the key is optional and defaults to `hundred`; once the mode exists the default follows it: school gives `twelve`, university gives `hundred`.
- `twelve`: an assignment's score is itself the grade, so every assignment has `max_points <= 12`. `is_bonus` has no effect.
- `hundred`: the course grade is the sum of its points. The sum of `max_points` over the regular assignments of a course (`is_bonus = false`) is at most 100. Drafts and `no_submission` assignments count. A sum below 100 is allowed while the course is being built. Bonus assignments do not count toward that sum and the scale does not limit them. Whether a course grade above 100 is capped is decided by the ADR that adds final grades.
- `content` checks the limit in its service when an assignment is created, when `max_points` is raised, and when `is_bonus` changes from true to false; lowering `max_points`, deleting an assignment, or turning a regular assignment into a bonus one never violates it. A violation is 422 on `max_points` (or on `is_bonus` when that is what changed), and the message states the limit and, for `hundred`, the points still free.
- For `hundred` the service first takes `pg_advisory_xact_lock` on the course id, then reads the sum in a later statement, so the second of two racing writers sees the first one's committed row (READ COMMITTED).
- Changing the setting does not revalidate existing assignments. The limit applies from the next write; an assignment already over it keeps working and can only be lowered.
- This ADR does not define how points become a value of the scale, rounding, the A–F table, or where final grades live. The ADR that adds final grades does. Letters are computed, never stored.

## Consequences
- Every course in one installation uses the same scale. Per-course scales need a new ADR that supersedes this one.
- At a university the course grade is a plain sum of points. Categories and weights would change the sum rule and need their own ADR.
- Nothing has to be removed from the database: the table was never built.
- ADR-0004 lists "change the grading scale" among the root actions; the scale is now a config setting, so that root action no longer exists. The other root actions are unchanged.
- The advisory lock is the first of its kind in this codebase; other modules do not use it.
- ARCHITECTURE.md loses the table, the column and the "change grading scale" permission.
- The `org` module shrinks to terms, groups and institution settings.
