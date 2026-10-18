CREATE TABLE events.audit_log (
    id            uuid        PRIMARY KEY,
    actor_id      uuid        NOT NULL REFERENCES identity.users (id),
    action        text        NOT NULL,
    resource_type text        NOT NULL,
    resource_id   uuid        NOT NULL,
    at            timestamptz NOT NULL DEFAULT now(),
    meta          jsonb       NOT NULL DEFAULT '{}'
);
CREATE INDEX audit_log_resource_idx ON events.audit_log (resource_type, resource_id, at);
