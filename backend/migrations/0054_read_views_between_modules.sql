-- ADR-0093. What a module of `learning` may read of the modules below it: the owner declares the
-- columns, nothing above reads the table itself (the rule of ADR-0065, now between the four
-- modules). Plain views on one table, as in 0033, so Postgres reads through them and row locks
-- taken through them lock the table's rows. A view leaves out what the readers above must not
-- see: the text of an item (its working copy), the content of a submission, and every draft
-- column of a grade.

-- teaching
CREATE VIEW teaching.course_list AS
SELECT id, kind, title, status, owner_id, term_id, code, description, created_at, updated_at
FROM teaching.courses;

CREATE VIEW teaching.announcement_list AS
SELECT id, course_id, author_id, title, body, created_at, updated_at FROM teaching.announcements;

CREATE VIEW teaching.tombstone_list AS
SELECT kind, id, course_id, deleted_at FROM teaching.tombstones;

-- content: the structure of a course and what an assignment asks. Not the body of an item; `edited`
-- says that a published item has a working copy students do not read yet (ADR-0049).
CREATE VIEW content.module_list AS
SELECT id, course_id, parent_id, position, title, status, publish_at, created_at, updated_at
FROM content.modules;

CREATE VIEW content.item_list AS
SELECT id, module_id, course_id, position, kind, title, status, publish_at, published_title,
       created_at, updated_at, last_published_at, announced_at,
       (status = 'published'
        AND (title IS DISTINCT FROM published_title OR body IS DISTINCT FROM published_body))
           AS edited
FROM content.items;

CREATE VIEW content.assignment_list AS
SELECT item_id, type, config, max_points, is_bonus, due_at, on_deadline, accept_late, late_until,
       max_attempts, class_on, created_at, updated_at
FROM content.assignments;

CREATE VIEW content.item_file_list AS
SELECT item_id, file_id, published FROM content.item_files;

-- assessment: which attempts and drafts exist (no content), and the submitted attempts with
-- their content. A reader above never gets the content of a draft.
CREATE VIEW assessment.submission_index AS
SELECT id, assignment_id, student_id, status, attempt, submitted_at, is_late, created_at, updated_at
FROM assessment.submissions;

CREATE VIEW assessment.submitted_attempts AS
SELECT id, assignment_id, student_id, attempt, content, submitted_at, is_late, auto_scores,
       auto_total, created_at, updated_at
FROM assessment.submissions
WHERE status = 'submitted';

CREATE VIEW assessment.submission_file_list AS
SELECT submission_id, file_id FROM assessment.submission_files;

-- gradebook: published values only. A draft score or feedback is not in the view, so a reader
-- above `gradebook` cannot select one (CLAUDE.md section 6); staff reads of drafts go through
-- `gradebook` functions.
CREATE VIEW gradebook.published_grades AS
SELECT assignment_id, student_id, published_score, published_feedback, published_by, published_at
FROM gradebook.grades
WHERE published_at IS NOT NULL OR published_score IS NOT NULL OR published_feedback IS NOT NULL;
