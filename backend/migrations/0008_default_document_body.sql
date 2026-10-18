-- ADR-0025. The body of a text or assignment item is a block document; the empty one is the default.
-- Rows that still hold the old empty object become the empty document. The object CHECK stays.
ALTER TABLE content.items
    ALTER COLUMN body SET DEFAULT '{"v": 1, "blocks": [], "attachments": []}';
UPDATE content.items
    SET body = '{"v": 1, "blocks": [], "attachments": []}'
    WHERE body = '{}'::jsonb;
