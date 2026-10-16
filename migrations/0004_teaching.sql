CREATE TABLE teaching.courses (
    id         uuid        PRIMARY KEY,
    kind       text        NOT NULL DEFAULT 'regular' CHECK (kind IN ('regular', 'template')),
    title      text        NOT NULL CHECK (title = btrim(title) AND char_length(title) BETWEEN 1 AND 200),
    term_id    uuid,       -- the foreign key to org.terms is added together with that table
    status     text        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published', 'archived')),
    owner_id   uuid        NOT NULL REFERENCES identity.users (id),
    created_by uuid        NOT NULL REFERENCES identity.users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (kind = 'regular' OR term_id IS NULL)
);

CREATE TABLE teaching.memberships (
    course_id  uuid        NOT NULL REFERENCES teaching.courses (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES identity.users (id),
    role       text        NOT NULL CHECK (role IN ('teacher', 'assistant', 'student')),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (course_id, user_id)
);
CREATE INDEX memberships_user_id_idx ON teaching.memberships (user_id, course_id);
