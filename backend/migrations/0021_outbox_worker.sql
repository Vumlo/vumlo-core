-- ADR-0045. The worker claims a row with a lease (run_after moves forward), runs the handler with no
-- transaction open, and then sets processed_at. A failure sets last_error and a later run_after; the
-- fifth failed attempt sets failed_at and the row stays as a dead letter.
ALTER TABLE events.outbox
    ADD COLUMN attempts  integer     NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    ADD COLUMN run_after timestamptz NOT NULL DEFAULT now(),
    ADD COLUMN last_error text,
    ADD COLUMN failed_at timestamptz,
    ADD CONSTRAINT outbox_processed_or_failed CHECK (processed_at IS NULL OR failed_at IS NULL);

-- The claim reads the oldest pending rows that are due.
CREATE INDEX outbox_pending_idx ON events.outbox (id) WHERE processed_at IS NULL AND failed_at IS NULL;
