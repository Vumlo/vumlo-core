-- ADR-0098, stage 4. Every item is an activity of a registered type.
--
-- `content.items.type` is the id of the activity type of the item: `lecture` for what was an item
-- of kind `text`, `shared_file` for kind `file`, and for an assignment the type that its row in
-- `content.assignments` had. The column `kind` stays and now says only what is structural: an item
-- is a `resource` (no points) or an `assignment` (it has a row in `content.assignments`, and the
-- foreign key of that table still makes this true). The type moves off `content.assignments`, so
-- the two CHECKs that named `no_submission` go with the column; the service enforces the same
-- rules through the capabilities of the type (a deadline or a class day only for a type that
-- takes submissions, ADR-0035, ADR-0049).

-- The views read what is being changed; they are recreated below with the same columns plus `type`.
DROP VIEW content.assignment_list;
DROP VIEW content.item_list;

ALTER TABLE content.items ADD COLUMN type text;

UPDATE content.items i
SET type = a.type
FROM content.assignments a
WHERE a.item_id = i.id;
UPDATE content.items SET type = 'lecture' WHERE kind = 'text';
UPDATE content.items SET type = 'shared_file' WHERE kind = 'file';

ALTER TABLE content.items
    ALTER COLUMN type SET NOT NULL,
    ADD CONSTRAINT items_type_check CHECK (type ~ '^[a-z][a-z0-9_]{0,31}$');

ALTER TABLE content.items DROP CONSTRAINT items_kind_check;
UPDATE content.items SET kind = 'resource' WHERE kind IN ('text', 'file');
ALTER TABLE content.items
    ADD CONSTRAINT items_kind_check CHECK (kind IN ('resource', 'assignment'));

ALTER TABLE content.assignments
    DROP CONSTRAINT assignments_no_submission_settings,
    DROP CONSTRAINT assignments_class_on_check,
    DROP COLUMN type;

CREATE VIEW content.item_list AS
SELECT id, module_id, course_id, position, kind, type, title, status, publish_at, published_title,
       created_at, updated_at, last_published_at, announced_at,
       (status = 'published'
        AND (title IS DISTINCT FROM published_title OR body IS DISTINCT FROM published_body))
           AS edited
FROM content.items;

CREATE VIEW content.assignment_list AS
SELECT a.item_id, i.type, a.config, a.max_points, a.is_bonus, a.due_at, a.on_deadline,
       a.accept_late, a.late_until, a.max_attempts, a.class_on, a.created_at, a.updated_at
FROM content.assignments a
JOIN content.items i ON i.id = a.item_id;
