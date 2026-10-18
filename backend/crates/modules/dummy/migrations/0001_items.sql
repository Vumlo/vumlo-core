-- The dummy module's own schema. It lives in its own migrations table
-- (_sqlx_migrations_dummy), so it never touches the core's.
CREATE SCHEMA dummy;

CREATE TABLE dummy.items (
    id         uuid        PRIMARY KEY,
    label      text        NOT NULL CHECK (char_length(label) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE dummy.heartbeat (
    id uuid        PRIMARY KEY,
    at timestamptz NOT NULL DEFAULT now()
);
