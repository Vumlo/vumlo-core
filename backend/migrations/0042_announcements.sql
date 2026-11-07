-- Course announcements (ADR-0076): a teacher tells a course something. Created is published: there is
-- no draft. Plain text, so nothing here needs a document model.
CREATE TABLE teaching.announcements (
    id         uuid        PRIMARY KEY,
    course_id  uuid        NOT NULL REFERENCES teaching.courses (id) ON DELETE CASCADE,
    author_id  uuid        NOT NULL REFERENCES auth.users (id),
    title      text        NOT NULL CHECK (title = btrim(title) AND char_length(title) BETWEEN 1 AND 200),
    body       text        NOT NULL CHECK (char_length(body) BETWEEN 1 AND 10000),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- A course's announcements, newest first (ids are UUID v7, so id order is time order).
CREATE INDEX announcements_by_course ON teaching.announcements (course_id, id DESC);
-- A plugin catches up by change time.
CREATE INDEX announcements_by_update ON teaching.announcements (updated_at, id);
