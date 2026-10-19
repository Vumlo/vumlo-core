-- ADR-0058. One row per `Idempotency-Key` a client sent with a POST. The key is scoped to the credential that
-- sent it (the hash of the bearer token or session cookie, never the token), so two people cannot meet each
-- other's keys. `status` is null while the first request runs; a finished one keeps its status, content type
-- and body for a day, and a retry with the same key and the same request gets exactly that answer.
CREATE SCHEMA idempotency;

CREATE TABLE idempotency.keys (
    scope_hash   bytea       NOT NULL CHECK (octet_length(scope_hash) = 32),
    key          text        NOT NULL CHECK (char_length(key) BETWEEN 1 AND 255),
    request_hash bytea       NOT NULL CHECK (octet_length(request_hash) = 32),
    status       smallint    CHECK (status BETWEEN 200 AND 499),
    content_type text,
    body         bytea,
    started_at   timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz NOT NULL DEFAULT now() + interval '24 hours',
    PRIMARY KEY (scope_hash, key),
    CHECK ((status IS NULL) = (body IS NULL))
);
CREATE INDEX keys_expires_idx ON idempotency.keys (expires_at);
