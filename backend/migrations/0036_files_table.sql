-- The description of a stored file (ADR-0069). The bytes live in the storage backend under a key
-- derived from the id. No foreign key to another schema: `files` sits below the layers that use it.
CREATE TABLE files.files (
    id          uuid PRIMARY KEY,
    uploaded_by uuid NOT NULL,
    name        text NOT NULL,
    mime        text NOT NULL,
    size        bigint NOT NULL CHECK (size > 0),
    sha256      text NOT NULL,
    created_at  timestamptz NOT NULL DEFAULT now()
);

-- The orphan sweep reads the oldest rows first.
CREATE INDEX files_created_at_idx ON files.files (created_at);
