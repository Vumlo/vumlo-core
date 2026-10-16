-- ADR-0007. A state change and its event are inserted in one transaction. The worker that claims
-- and delivers rows (and the index it needs) arrives in its own task.
CREATE TABLE events.outbox (
    id           uuid        PRIMARY KEY,
    type         text        NOT NULL,
    version      integer     NOT NULL DEFAULT 1 CHECK (version >= 1),
    payload      jsonb       NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    occurred_at  timestamptz NOT NULL DEFAULT now(),
    processed_at timestamptz
);
