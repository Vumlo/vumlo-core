-- ADR-0034. An audit row can come from the server CLI, where nobody is signed in: it has no actor
-- and the source `cli`. Every other row keeps its actor and has the source `api`.
ALTER TABLE events.audit_log
    ALTER COLUMN actor_id DROP NOT NULL,
    ADD COLUMN source text NOT NULL DEFAULT 'api' CHECK (source IN ('api', 'cli')),
    ADD CONSTRAINT audit_log_actor_source_check CHECK ((source = 'api') = (actor_id IS NOT NULL));
