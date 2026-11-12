-- ADR-0093. What `content` knows about a student, kept in `content` itself.
--
-- The access rules (ADR-0092) ask: has the student done this item, and what is their published
-- score on that assignment? The answers belong to `assessment` (submissions) and `gradebook`
-- (grades), which sit above `content`. So `content` keeps its own copy of the facts it needs, one
-- row per student and item, updated in the transaction of the change by event subscribers
-- (`learning::facts`), and rebuilt from the sources of truth by `vumlo-cli facts rebuild`.
--
-- `marked_at` is not a copy: the student's own mark on a text or file item is written here and
-- nowhere else (it replaces `content.completions`, dropped by the next migration).
CREATE TABLE content.facts (
    item_id         uuid          NOT NULL REFERENCES content.items (id) ON DELETE CASCADE,
    student_id      uuid          NOT NULL REFERENCES auth.users (id),
    -- The first submitted attempt of an assignment.
    submitted_at    timestamptz,
    -- The score the student can see: the published one, never a draft.
    published_score numeric(6,2)  CHECK (published_score >= 0),
    -- The student marked a text or file item done.
    marked_at       timestamptz,
    PRIMARY KEY (item_id, student_id),
    CHECK (submitted_at IS NOT NULL OR published_score IS NOT NULL OR marked_at IS NOT NULL)
);

-- A student's facts in a course are read together, for the course tree.
CREATE INDEX facts_student_idx ON content.facts (student_id);

INSERT INTO content.facts (item_id, student_id, submitted_at, published_score, marked_at)
SELECT item_id, student_id, min(submitted_at), max(published_score), max(marked_at)
FROM (
    SELECT assignment_id AS item_id, student_id, submitted_at,
           NULL::numeric AS published_score, NULL::timestamptz AS marked_at
    FROM assessment.submissions
    WHERE status = 'submitted'
    UNION ALL
    SELECT assignment_id, student_id, NULL, published_score, NULL
    FROM gradebook.grades
    WHERE published_score IS NOT NULL
    UNION ALL
    SELECT item_id, student_id, NULL, NULL, at
    FROM content.completions
) sources
GROUP BY item_id, student_id;
