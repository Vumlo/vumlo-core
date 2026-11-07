-- What a plugin did to the core (ADR-0079). Two records, for two readers.
--
-- 1. `plugins.api_calls`: a line per call a plugin made to the plugin API, for the administrator who
--    wonders what a plugin is doing: when, which route (the template, never the ids in it), the
--    answer and how long it took. No body, no query, no person. Old lines are deleted by the
--    delivery worker.
CREATE TABLE plugins.api_calls (
    id          uuid        PRIMARY KEY,
    install_id  uuid        NOT NULL REFERENCES plugins.plugin_installs (id) ON DELETE CASCADE,
    at          timestamptz NOT NULL DEFAULT now(),
    method      text        NOT NULL,
    route       text        NOT NULL,
    status      smallint    NOT NULL,
    duration_ms integer     NOT NULL CHECK (duration_ms >= 0)
);

CREATE INDEX api_calls_by_install ON plugins.api_calls (install_id, at DESC);
CREATE INDEX api_calls_by_time ON plugins.api_calls (at);

-- 2. The audit log: what a plugin changed. Nobody is signed in and nobody is the actor, as with the
--    CLI; the row says which plugin in `meta`.
ALTER TABLE events.audit_log DROP CONSTRAINT audit_log_source_check;
ALTER TABLE events.audit_log
    ADD CONSTRAINT audit_log_source_check CHECK (source IN ('api', 'cli', 'plugin'));
