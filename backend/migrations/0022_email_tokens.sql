-- ADR-0046. The tokens of the verification and reset links. Only the SHA-256 of a token is stored;
-- the raw value exists only in the mail. A token is used once and expires.
CREATE TABLE identity.email_tokens (
    id         uuid        PRIMARY KEY,
    user_id    uuid        NOT NULL REFERENCES identity.users (id) ON DELETE CASCADE,
    purpose    text        NOT NULL CHECK (purpose IN ('verify', 'reset')),
    token_hash bytea       NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    used_at    timestamptz,
    CHECK (expires_at > created_at)
);
-- "Void the unused tokens of this user and purpose".
CREATE INDEX email_tokens_user_idx ON identity.email_tokens (user_id, purpose) WHERE used_at IS NULL;

-- Every account that exists was created by an admin, the CLI or a test: its address was given by
-- someone who vouches for it, so it counts as verified (ADR-0046).
UPDATE identity.users SET email_verified_at = created_at WHERE email_verified_at IS NULL;
