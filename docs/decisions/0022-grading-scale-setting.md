# 0022. The grading scale is an institution setting

Status: Accepted
Supplements: ADR-0002, ADR-0009
Amends: ADR-0004

## Context
ARCHITECTURE.md planned an `org.grading_scales` table and a `courses.grading_scale_id` column, for scales such as 12-point and 100-point with ECTS. The real need is smaller. One installation serves one institution (ADR-0002), schools grade 1–12 and universities 1–100, and letter grades (A–F) are a fixed function of the score. Final grades are outside v1 (ADR-0009), and assignment points are independent of any scale (ADR-0021).

## Options
1. A table of scales referenced from each course — flexible, but nobody asked for per-course scales, and it costs a migration, seed data and a backfill.
2. **A setting `institution.grading_scale` in the config file** (`twelve` | `hundred`), defaulting from `institution.mode`: school gives `twelve`, university gives `hundred`.
3. Hard-code the scale to the mode — a school that wants 100 points has no way to get it.

## Decision
- There is no `org.grading_scales` table and no `courses.grading_scale_id`. Both are removed from the design.
- The scale is `institution.grading_scale = twelve | hundred` in `learnly.toml`, defaulting by `institution.mode`. Neither key exists yet; they arrive with the first feature that needs them.
- The scale matters only for final grades and for letter display. A pure function maps points and a maximum to a scale value; letters are computed, never stored. Assignment scores are in points up to `max_points` and do not depend on the scale.

## Consequences
- Every course in one installation uses the same scale. If an institution ever needs per-course scales, a new ADR supersedes this one and adds the table then.
- Nothing has to be removed from the database: the table was never built.
- ARCHITECTURE.md loses the table, the column and the "change grading scale" permission.
- ADR-0004 lists "change the grading scale" among the root actions; with no scale to change, that root action no longer exists. The other root actions are unchanged.
- The `org` module shrinks to terms, groups and institution settings.
