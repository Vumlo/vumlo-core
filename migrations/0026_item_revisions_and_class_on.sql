-- ADR-0049. An item has a working title and body (what staff edit) and a published title and body
-- (what students read). The published pair is empty until the item is published first.
ALTER TABLE content.items
    ADD COLUMN published_title    text,
    ADD COLUMN published_body     jsonb CHECK (jsonb_typeof(published_body) = 'object'),
    ADD COLUMN last_published_at  timestamptz;

-- What is published now is what students have been reading: the current title and body.
UPDATE content.items
SET published_title = title, published_body = body, last_published_at = updated_at
WHERE status = 'published';

ALTER TABLE content.items
    ADD CONSTRAINT items_published_slot_check
        CHECK (status <> 'published'
               OR (published_title IS NOT NULL AND published_body IS NOT NULL
                   AND last_published_at IS NOT NULL));

-- The day an in-class (`no_submission`) assignment takes place. Such an assignment has no deadline.
ALTER TABLE content.assignments
    ADD COLUMN class_on date,
    ADD CONSTRAINT assignments_class_on_check CHECK (class_on IS NULL OR type = 'no_submission');
