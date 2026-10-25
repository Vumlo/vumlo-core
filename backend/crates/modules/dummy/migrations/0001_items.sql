-- The dummy module's own schema, `mod_dummy`, is created by its migrator (module_api::scoped_migrator),
-- which also keeps the history in mod_dummy._sqlx_migrations. The core is never touched.
CREATE TABLE mod_dummy.items (
    id         uuid        PRIMARY KEY,
    label      text        NOT NULL CHECK (char_length(label) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE mod_dummy.heartbeat (
    id uuid        PRIMARY KEY,
    at timestamptz NOT NULL DEFAULT now()
);

-- The outbox events the module has handled (ids only), to show that a module's handler runs.
CREATE TABLE mod_dummy.seen_events (
    event_id uuid        PRIMARY KEY,
    at       timestamptz NOT NULL DEFAULT now()
);
