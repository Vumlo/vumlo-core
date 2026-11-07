-- The Buddy's own schema, `mod_ai_agent`, is created by its migrator (module_api::scoped_migrator).
-- No foreign key to the core: a module names only its own schema (ADR-0066). The retention job
-- deletes what a removed user leaves behind.

-- The visible chat only: what the user wrote and what the Buddy answered. Tool calls and their
-- results live in memory for one turn and are never stored (they carry grades and names).
-- Ids are UUID v7, so `ORDER BY id` is the order in time.
CREATE TABLE mod_ai_agent.messages (
    id         uuid        PRIMARY KEY,
    user_id    uuid        NOT NULL,
    role       text        NOT NULL CHECK (role IN ('user', 'assistant')),
    content    text        NOT NULL CHECK (char_length(content) BETWEEN 1 AND 20000),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX messages_by_user ON mod_ai_agent.messages (user_id, id);
CREATE INDEX messages_by_age ON mod_ai_agent.messages (created_at);

-- Messages a user has sent today, for the daily cap.
CREATE TABLE mod_ai_agent.usage (
    user_id uuid    NOT NULL,
    day     date    NOT NULL,
    count   integer NOT NULL DEFAULT 0 CHECK (count >= 0),
    PRIMARY KEY (user_id, day)
);
