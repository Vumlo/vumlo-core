-- ADR-0024. The single-document item kind `lesson` is now `text`; `lesson` is free for ADR-0026.
-- The old CHECK refuses 'text', so it goes first; adding the new one validates every row.
ALTER TABLE content.items DROP CONSTRAINT items_kind_check;
UPDATE content.items SET kind = 'text' WHERE kind = 'lesson';
ALTER TABLE content.items ADD CONSTRAINT items_kind_check CHECK (kind IN ('text', 'assignment'));
