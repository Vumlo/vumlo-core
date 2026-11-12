-- ADR-0091. The two jobs of the scheduler (`learning::schedule`) each leave a mark on the row they
-- work on, so a tick that runs twice, or two workers, do the work once.

-- `assignment.created` is sent when an assignment becomes visible to students for the first time,
-- which is not the moment it is published: the module or the course may be a draft, and
-- `publish_at` may lie ahead (ADR-0075). The scheduler and the publish routes send it for every
-- published assignment that is visible now and has no `announced_at`, and set the column.
-- Every assignment published before this migration was announced when it was published.
ALTER TABLE content.items ADD COLUMN announced_at timestamptz;

UPDATE content.items
SET announced_at = COALESCE(last_published_at, updated_at)
WHERE kind = 'assignment' AND status = 'published';

-- Only what is still to be announced is in the index, so it stays small.
CREATE INDEX items_unannounced_idx ON content.items (course_id)
    WHERE kind = 'assignment' AND status = 'published' AND announced_at IS NULL;

-- `on_deadline = auto_submit` (ADR-0035): at `due_at` the scheduler submits each student's draft.
-- A draft that the submit refuses (blank, a required field missing, the attempt limit reached) is
-- kept and marked here, so the next tick does not try it again; the student's own work is
-- untouched. If the student then submits it by hand, the row keeps the mark; nothing reads it
-- for a submitted row.
ALTER TABLE assessment.submissions ADD COLUMN auto_submit_skipped_at timestamptz;

-- The scheduler's lookup: drafts not yet tried.
CREATE INDEX submissions_auto_submit_idx ON assessment.submissions (assignment_id)
    WHERE status = 'draft' AND auto_submit_skipped_at IS NULL;
