-- ADR-0035. The drafts and the submitted attempts of students. A submit turns the draft row into
-- attempt n+1 in place. assignment_id has no foreign key (ADR-0021: the assignment lives in another
-- schema); content calls assessment's guards and purge before it deletes an assignment.
CREATE TABLE assessment.submissions (
    id            uuid        PRIMARY KEY,
    assignment_id uuid        NOT NULL,
    student_id    uuid        NOT NULL REFERENCES identity.users (id),
    status        text        NOT NULL CHECK (status IN ('draft', 'submitted')),
    -- Numbered from 1 per student and assignment; a draft has none.
    attempt       integer     CHECK (attempt >= 1),
    -- A block document of the student profile (ADR-0025).
    content       jsonb       NOT NULL CHECK (jsonb_typeof(content) = 'object'),
    submitted_at  timestamptz,
    is_late       boolean     NOT NULL DEFAULT false,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    -- A draft has no number and no submit time; a submitted row has both.
    CHECK (status <> 'draft' OR (attempt IS NULL AND submitted_at IS NULL)),
    CHECK (status <> 'submitted' OR (attempt IS NOT NULL AND submitted_at IS NOT NULL)),
    CHECK (status = 'submitted' OR NOT is_late)
);

-- At most one draft per (assignment, student); also the target of the draft upsert.
CREATE UNIQUE INDEX submissions_one_draft_idx
    ON assessment.submissions (assignment_id, student_id) WHERE status = 'draft';
-- The numbering of attempts, and the lookup of the latest one.
CREATE UNIQUE INDEX submissions_attempt_idx
    ON assessment.submissions (assignment_id, student_id, attempt) WHERE status = 'submitted';
CREATE INDEX submissions_student_idx ON assessment.submissions (student_id);

-- A submitted row is never changed. Deleting stays possible for the purge of drafts (ADR-0035).
CREATE FUNCTION assessment.refuse_submitted_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.status = 'submitted' THEN
        RAISE EXCEPTION 'a submitted attempt is never changed';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER submissions_submitted_is_final BEFORE UPDATE ON assessment.submissions
    FOR EACH ROW EXECUTE FUNCTION assessment.refuse_submitted_update();
