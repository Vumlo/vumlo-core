-- ADR-0037. "Does this user own a course?" runs on every demotion, and "my courses" asks for the
-- courses an owner has.
CREATE INDEX courses_owner_id_idx ON teaching.courses (owner_id);
