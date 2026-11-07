-- A plugin is told when the change happened, not only when the delivery was made (ADR-0075). The
-- time is the `occurred_at` of the outbox event; a delivery made before this migration gets the
-- time it was owed.
ALTER TABLE plugins.deliveries ADD COLUMN occurred_at timestamptz;
UPDATE plugins.deliveries SET occurred_at = created_at;
ALTER TABLE plugins.deliveries ALTER COLUMN occurred_at SET NOT NULL;
