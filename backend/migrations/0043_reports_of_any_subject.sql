-- A plugin reports on more than a submitted attempt: on an assignment, or on a course (ADR-0077). A
-- report is still one row per plugin, subject and key, replaced by the next with the same key. The
-- subject is exactly one of three ids, each with its own cascading key, so a report goes with what it
-- is about; `subject_id` is whichever it is, for the primary key and for reading.
ALTER TABLE plugins.reports DROP CONSTRAINT reports_pkey;

ALTER TABLE plugins.reports
    ALTER COLUMN submission_id DROP NOT NULL,
    ADD COLUMN assignment_id uuid REFERENCES content.assignments (item_id) ON DELETE CASCADE,
    ADD COLUMN course_id     uuid REFERENCES teaching.courses (id) ON DELETE CASCADE,
    -- A name the plugin gives the report, so that it may keep several on one subject. What it
    -- reported before this migration is `default`.
    ADD COLUMN key           text NOT NULL DEFAULT 'default'
        CHECK (key ~ '^[a-z][a-z0-9_-]{0,39}$'),
    ADD CONSTRAINT reports_one_subject
        CHECK (num_nonnulls(submission_id, assignment_id, course_id) = 1);

ALTER TABLE plugins.reports ALTER COLUMN key DROP DEFAULT;

ALTER TABLE plugins.reports
    ADD COLUMN subject_id uuid GENERATED ALWAYS AS (COALESCE(submission_id, assignment_id, course_id)) STORED;

ALTER TABLE plugins.reports ADD PRIMARY KEY (install_id, subject_id, key);

DROP INDEX plugins.reports_by_submission;
CREATE INDEX reports_by_subject ON plugins.reports (subject_id);
