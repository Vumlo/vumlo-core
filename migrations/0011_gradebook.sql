-- ADR-0028. One row per (assignment, student) with a draft slot and a published slot, and an
-- append-only history. assignment_id has no foreign key (ADR-0021: the assignment lives in another
-- schema); content calls the gradebook's guards and purge before it deletes an assignment.
CREATE TABLE gradebook.grades (
    assignment_id      uuid          NOT NULL,
    student_id         uuid          NOT NULL REFERENCES identity.users (id),
    -- The upper bound is the assignment's max_points, checked by the service (no CHECK spans schemas).
    draft_score        numeric(6,2)  CHECK (draft_score >= 0),
    draft_feedback     text          CHECK (draft_feedback = btrim(draft_feedback)
                                            AND char_length(draft_feedback) BETWEEN 1 AND 4000),
    draft_updated_by   uuid          NOT NULL REFERENCES identity.users (id),
    draft_updated_at   timestamptz   NOT NULL DEFAULT now(),
    published_score    numeric(6,2)  CHECK (published_score >= 0),
    published_feedback text          CHECK (published_feedback = btrim(published_feedback)
                                            AND char_length(published_feedback) BETWEEN 1 AND 4000),
    published_by       uuid          REFERENCES identity.users (id),
    published_at       timestamptz,
    CHECK ((published_by IS NULL) = (published_at IS NULL)),
    PRIMARY KEY (assignment_id, student_id)
);
-- "My grades" reads by student.
CREATE INDEX grades_student_idx ON gradebook.grades (student_id, assignment_id);

CREATE TABLE gradebook.grade_history (
    id            uuid          PRIMARY KEY,
    assignment_id uuid          NOT NULL,
    student_id    uuid          NOT NULL REFERENCES identity.users (id),
    action        text          NOT NULL CHECK (action IN ('save_draft', 'publish')),
    actor_id      uuid          NOT NULL REFERENCES identity.users (id),
    at            timestamptz   NOT NULL DEFAULT now(),
    -- The values after the action: the draft for a save, the published value for a publish.
    score         numeric(6,2)  CHECK (score >= 0),
    feedback      text          CHECK (feedback = btrim(feedback) AND char_length(feedback) BETWEEN 1 AND 4000)
);
CREATE INDEX grade_history_cell_idx ON gradebook.grade_history (assignment_id, student_id, at);

-- Append-only: a row is never changed. Deleting stays possible for the purge that goes with the
-- deletion of an assignment (ADR-0028).
CREATE FUNCTION gradebook.refuse_history_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'gradebook.grade_history is append-only';
END;
$$;
CREATE TRIGGER grade_history_no_update BEFORE UPDATE ON gradebook.grade_history
    FOR EACH ROW EXECUTE FUNCTION gradebook.refuse_history_update();
