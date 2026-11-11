-- The mail layer (ADR-0084 to ADR-0086): every outgoing mail is a row, one per recipient, sent by one
-- worker. No foreign key to another schema: `mail` sits below `auth`, and a user id is only a number
-- here. A recipient's address is copied into the row and erased after `mail.retention_days`.
CREATE SCHEMA mail;

-- One per enqueue. `plugin_id` is NULL for the core's own account mail. The subject, the text and the
-- link are removed once the last address of the batch is erased.
CREATE TABLE mail.batches (
    id         uuid        PRIMARY KEY,
    plugin_id  text,
    kind       text        NOT NULL CHECK (kind ~ '^[a-z][a-z0-9_-]{0,39}$'),
    subject    text,
    body       text,
    link_path  text,
    accepted   integer     NOT NULL DEFAULT 0 CHECK (accepted >= 0),
    skipped    jsonb       NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(skipped) = 'object'),
    created_at timestamptz NOT NULL DEFAULT now()
);
-- The hourly limit of a plugin counts its batches of the last hour.
CREATE INDEX batches_by_plugin ON mail.batches (plugin_id, created_at) WHERE plugin_id IS NOT NULL;
CREATE INDEX batches_by_time ON mail.batches (created_at);

CREATE TABLE mail.messages (
    id          uuid        PRIMARY KEY,
    batch_id    uuid        NOT NULL REFERENCES mail.batches (id) ON DELETE CASCADE,
    user_id     uuid        NOT NULL,
    plugin_id   text,
    kind        text        NOT NULL CHECK (kind ~ '^[a-z][a-z0-9_-]{0,39}$'),
    to_address  text,
    to_name     text,
    state       text        NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'sent', 'dead', 'cancelled')),
    attempts    integer     NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    run_after   timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL DEFAULT now() + interval '24 hours',
    -- A class such as `smtp 451` or `connect`, never the server's text (it names the address).
    last_error  text,
    finished_at timestamptz,
    erase_at    timestamptz,
    -- The outbox event an account mail answers: a rerun of its handler queues it once.
    origin_id   uuid        UNIQUE,
    created_at  timestamptz NOT NULL DEFAULT now(),
    CHECK (state = 'pending' OR finished_at IS NOT NULL),
    CHECK (state <> 'pending' OR to_address IS NOT NULL)
);
CREATE INDEX messages_due ON mail.messages (run_after) WHERE state = 'pending';
CREATE INDEX messages_to_erase ON mail.messages (erase_at) WHERE to_address IS NOT NULL AND erase_at IS NOT NULL;
CREATE INDEX messages_by_user ON mail.messages (user_id, created_at) WHERE plugin_id IS NOT NULL;
CREATE INDEX messages_by_batch ON mail.messages (batch_id);
CREATE INDEX messages_by_time ON mail.messages (created_at);

-- A person's choice: no mail of this kind from this plugin. Kept when the plugin is removed.
CREATE TABLE mail.opt_outs (
    user_id    uuid        NOT NULL,
    plugin_id  text        NOT NULL,
    kind       text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, plugin_id, kind)
);

-- `(plugin, dedupe_key, person)` is accepted once in 30 days.
CREATE TABLE mail.dedupe (
    plugin_id  text        NOT NULL,
    dedupe_key text        NOT NULL,
    user_id    uuid        NOT NULL,
    at         timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (plugin_id, dedupe_key, user_id)
);
CREATE INDEX dedupe_by_time ON mail.dedupe (at);

-- One row: the key that signs the opt-out links. Made on first use.
CREATE TABLE mail.signing_key (
    id         boolean     PRIMARY KEY DEFAULT true CHECK (id),
    key        bytea       NOT NULL CHECK (octet_length(key) = 32),
    created_at timestamptz NOT NULL DEFAULT now()
);

-- `learning` mails only verified addresses (ADR-0085): the flag goes to the end of the view.
CREATE OR REPLACE VIEW auth.people AS
SELECT id, name, surname, email, role, status, (email_verified_at IS NOT NULL) AS email_verified
FROM auth.users;
