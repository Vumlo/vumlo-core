-- What a plugin that syncs by `updated_since` cannot see by itself: that something is gone (ADR-0081).
-- One row for each published assignment and each announcement that is deleted, written by the
-- database in the transaction of the delete, whatever deletes it (the item, its module, the course).
-- A draft assignment was never shown, so it leaves nothing. Attempts and grades have none: the guards
-- of ADR-0028 and ADR-0035 refuse to delete an assignment or a course that holds either. The delivery
-- worker deletes the rows older than `plugins.tombstone_days`.
CREATE TABLE teaching.tombstones (
    kind       text        NOT NULL CHECK (kind IN ('assignment', 'announcement')),
    id         uuid        NOT NULL,
    -- No foreign key: the course may be the thing that was deleted.
    course_id  uuid        NOT NULL,
    deleted_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (kind, id)
);
CREATE INDEX tombstones_by_time ON teaching.tombstones (kind, deleted_at, id);

CREATE FUNCTION teaching.tombstone_assignment() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.kind = 'assignment' AND OLD.status = 'published' THEN
        INSERT INTO teaching.tombstones (kind, id, course_id)
        VALUES ('assignment', OLD.id, OLD.course_id)
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN OLD;
END;
$$;
CREATE TRIGGER items_tombstone BEFORE DELETE ON content.items
    FOR EACH ROW EXECUTE FUNCTION teaching.tombstone_assignment();

CREATE FUNCTION teaching.tombstone_announcement() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO teaching.tombstones (kind, id, course_id)
    VALUES ('announcement', OLD.id, OLD.course_id)
    ON CONFLICT DO NOTHING;
    RETURN OLD;
END;
$$;
CREATE TRIGGER announcements_tombstone BEFORE DELETE ON teaching.announcements
    FOR EACH ROW EXECUTE FUNCTION teaching.tombstone_announcement();
