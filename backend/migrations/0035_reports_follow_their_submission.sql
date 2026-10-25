-- A plugin's report is about a submitted attempt and cannot outlive it (ADR-0066). Migration 0034 kept
-- the submission as a bare id, as grades keep their assignment (ADR-0021). That absence of a key is
-- there for grades so that a delete is refused with 409 instead of cascading; for a report the delete
-- should follow, in the same transaction, whichever way the attempt goes. Nothing deletes a submitted
-- attempt today (a course, a module or an assignment with one is refused); this makes it hold for the
-- delete that will, such as the erasure of a student's data.
--
-- A report cannot exist without its attempt, so there is nothing to clean first.
ALTER TABLE plugins.reports
    ADD CONSTRAINT reports_submission_fkey
    FOREIGN KEY (submission_id) REFERENCES assessment.submissions (id) ON DELETE CASCADE;
