-- ADR-0024, ADR-0069. A third item kind: `file`, one uploaded file with an optional description.
ALTER TABLE content.items DROP CONSTRAINT items_kind_check;
ALTER TABLE content.items
    ADD CONSTRAINT items_kind_check CHECK (kind IN ('text', 'assignment', 'file'));
