-- ADR-0047. One row per started Google sign-in. It is found by the hash of the `state` that comes back
-- in the callback, and only together with the hash of a random value that was set as a cookie in the
-- same browser, so a callback cannot be finished from another browser. A row is deleted when it is
-- read (single use) and when it has expired.
CREATE TABLE identity.oauth_states (
    state_hash    bytea       PRIMARY KEY CHECK (octet_length(state_hash) = 32),
    browser_hash  bytea       NOT NULL CHECK (octet_length(browser_hash) = 32),
    nonce         text        NOT NULL,
    pkce_verifier text        NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    CHECK (expires_at > created_at)
);
CREATE INDEX oauth_states_expires_idx ON identity.oauth_states (expires_at);
