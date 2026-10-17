-- ADR-0035. The deadline settings of an assignment that takes submissions (ADR-0011). All columns
-- are nullable: an existing assignment has no deadline and takes any number of attempts.
ALTER TABLE content.assignments
    ADD COLUMN due_at       timestamptz,
    ADD COLUMN on_deadline  text CHECK (on_deadline IN ('keep_draft', 'auto_submit')),
    ADD COLUMN accept_late  boolean,
    ADD COLUMN late_until   timestamptz,
    ADD COLUMN max_attempts integer CHECK (max_attempts >= 1),
    -- No hidden defaults: a deadline comes with what happens at it and whether late work is taken.
    ADD CONSTRAINT assignments_deadline_all_or_none
        CHECK ((due_at IS NULL) = (on_deadline IS NULL) AND (due_at IS NULL) = (accept_late IS NULL)),
    -- accept_late is set only together with due_at, so late_until is after a real deadline. IS TRUE:
    -- with accept_late or due_at null the comparison is null, and a CHECK lets null through.
    ADD CONSTRAINT assignments_late_until_check
        CHECK (late_until IS NULL OR (accept_late AND late_until > due_at) IS TRUE),
    -- A type that takes no submissions has no settings. The type is plain text (ADR-0008), so a
    -- later type of that kind needs its own migration.
    ADD CONSTRAINT assignments_no_submission_settings
        CHECK (type <> 'no_submission' OR (due_at IS NULL AND max_attempts IS NULL));
