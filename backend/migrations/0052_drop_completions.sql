-- ADR-0093. The student's own marks live in `content.facts.marked_at` now (copied there by the
-- previous migration), next to what `assessment` and `gradebook` say.
DROP TABLE content.completions;
