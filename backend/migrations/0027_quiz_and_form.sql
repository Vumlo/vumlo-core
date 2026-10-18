-- ADR-0050 and ADR-0051. A quiz is scored when it is submitted, and the score may become the draft
-- grade without a person behind it.

-- The score of each question of a quiz attempt: question id to a decimal string. It is set by the
-- statement that turns a draft into an attempt, so a submitted row still never changes.
ALTER TABLE assessment.submissions
    ADD COLUMN auto_scores jsonb CHECK (jsonb_typeof(auto_scores) = 'object'),
    ADD CONSTRAINT submissions_auto_scores_when_submitted CHECK (status = 'submitted' OR auto_scores IS NULL);

-- A draft grade written by the server has no person.
ALTER TABLE gradebook.grades ALTER COLUMN draft_updated_by DROP NOT NULL;

ALTER TABLE gradebook.grade_history ALTER COLUMN actor_id DROP NOT NULL;
ALTER TABLE gradebook.grade_history DROP CONSTRAINT grade_history_action_check;
ALTER TABLE gradebook.grade_history
    ADD CONSTRAINT grade_history_action_check CHECK (action IN ('save_draft', 'publish', 'auto_score')),
    ADD CONSTRAINT grade_history_actor_check CHECK ((action = 'auto_score') = (actor_id IS NULL));
