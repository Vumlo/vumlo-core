-- Plugins (ADR-0066): external HTTP services the core does not trust. The schema `plugins` belongs to
-- the crate of the same name. Nothing here has a foreign key to another schema: a report names a
-- submission by id only, as grades and submissions name an assignment (ADR-0021).

CREATE SCHEMA plugins;

-- One row per installed plugin. The token is shown once and only its SHA-256 is kept; the webhook
-- secret is not stored at all, it is derived from `plugins.secret_key`, the id and `secret_version`.
CREATE TABLE plugins.plugin_installs (
    id               uuid        PRIMARY KEY,
    plugin_id        text        NOT NULL UNIQUE CHECK (plugin_id ~ '^[a-z][a-z0-9_-]{0,31}$'),
    version          text        NOT NULL CHECK (char_length(version) BETWEEN 1 AND 32),
    base_url         text        NOT NULL CHECK (char_length(base_url) BETWEEN 1 AND 500),
    api              text        NOT NULL CHECK (api = '1'),
    -- Event types the plugin asked for, the scopes it asked for, and the scopes an administrator
    -- granted (never more than were asked for).
    subscribes       text[]      NOT NULL DEFAULT '{}',
    requested_scopes text[]      NOT NULL DEFAULT '{}',
    scopes           text[]      NOT NULL DEFAULT '{}',
    ui_slots         jsonb       NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(ui_slots) = 'array'),
    token_hash       bytea       NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    secret_version   integer     NOT NULL DEFAULT 1 CHECK (secret_version >= 1),
    status           text        NOT NULL DEFAULT 'enabled' CHECK (status IN ('enabled', 'disabled')),
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CHECK (scopes <@ requested_scopes)
);

-- An event owed to a plugin. Unique per install and event id: the fan-out can run twice.
CREATE TABLE plugins.deliveries (
    id           uuid        PRIMARY KEY,
    install_id   uuid        NOT NULL REFERENCES plugins.plugin_installs (id) ON DELETE CASCADE,
    event_id     uuid        NOT NULL,
    event_type   text        NOT NULL,
    -- The payload of the outbox event: ids only (AGENTS section 7).
    data         jsonb       NOT NULL,
    attempts     integer     NOT NULL DEFAULT 0,
    run_after    timestamptz NOT NULL DEFAULT now(),
    last_error   text,
    delivered_at timestamptz,
    failed_at    timestamptz,
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (install_id, event_id)
);

CREATE INDEX deliveries_pending ON plugins.deliveries (run_after)
    WHERE delivered_at IS NULL AND failed_at IS NULL;

-- What a plugin reported about a submitted attempt. One per plugin and attempt: a re-delivered
-- event replaces it.
CREATE TABLE plugins.reports (
    install_id    uuid        NOT NULL REFERENCES plugins.plugin_installs (id) ON DELETE CASCADE,
    submission_id uuid        NOT NULL,
    summary       text        NOT NULL CHECK (char_length(summary) BETWEEN 1 AND 500),
    data          jsonb       NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(data) = 'object'),
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (install_id, submission_id)
);

CREATE INDEX reports_by_submission ON plugins.reports (submission_id);
