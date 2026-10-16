-- ADR-0020. Modules form a tree of depth one inside a course; items (lessons) hang off a module.
-- Positions are unique per sibling sequence and checked at commit, so a reorder can rewrite them
-- in any order within one transaction.
CREATE TABLE content.modules (
    id         uuid        PRIMARY KEY,
    course_id  uuid        NOT NULL REFERENCES teaching.courses (id) ON DELETE CASCADE,
    parent_id  uuid,
    position   integer     NOT NULL CHECK (position >= 1),
    title      text        NOT NULL CHECK (title = btrim(title) AND char_length(title) BETWEEN 1 AND 200),
    status     text        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published')),
    publish_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    -- Depth one. A sub-module's parent_top_level is true, so its parent must be a row whose
    -- top_level is true, i.e. a module that has no parent itself. A top-level module has
    -- parent_id NULL and parent_top_level NULL, which the second foreign key skips.
    top_level        boolean NOT NULL GENERATED ALWAYS AS (parent_id IS NULL) STORED,
    parent_top_level boolean GENERATED ALWAYS AS (CASE WHEN parent_id IS NOT NULL THEN true END) STORED,
    UNIQUE (id, course_id),
    UNIQUE (id, top_level),
    -- The parent belongs to the same course.
    FOREIGN KEY (parent_id, course_id) REFERENCES content.modules (id, course_id) ON DELETE CASCADE,
    FOREIGN KEY (parent_id, parent_top_level) REFERENCES content.modules (id, top_level) ON DELETE CASCADE,
    -- NULL parents compare equal, so the top-level modules of a course are one sequence too.
    UNIQUE NULLS NOT DISTINCT (course_id, parent_id, position) DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE content.items (
    id         uuid        PRIMARY KEY,
    module_id  uuid        NOT NULL,
    course_id  uuid        NOT NULL,
    position   integer     NOT NULL CHECK (position >= 1),
    kind       text        NOT NULL CHECK (kind IN ('lesson')),
    title      text        NOT NULL CHECK (title = btrim(title) AND char_length(title) BETWEEN 1 AND 200),
    body       jsonb       NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(body) = 'object'),
    status     text        NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published')),
    publish_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (module_id, course_id) REFERENCES content.modules (id, course_id) ON DELETE CASCADE,
    UNIQUE (module_id, position) DEFERRABLE INITIALLY DEFERRED
);
CREATE INDEX items_course_id_idx ON content.items (course_id);
