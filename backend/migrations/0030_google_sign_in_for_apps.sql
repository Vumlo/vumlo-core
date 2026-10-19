-- ADR-0062. Sign-in with Google for a native app. The app starts the attempt itself (there is no browser
-- cookie to tie it to), so an attempt is tied either to a browser (`browser_hash`) or to the app that started it
-- (`app_challenge`, the SHA-256 of a secret that only the app holds). When Google sends the person back, the
-- web app's callback page finishes the attempt and a short-lived, single-use `handoff` code is delivered to the
-- app by a link; the app trades it, with the secret, for a session.
ALTER TABLE identity.oauth_states
    ALTER COLUMN browser_hash DROP NOT NULL,
    ADD COLUMN app_challenge bytea CHECK (octet_length(app_challenge) = 32),
    ADD CONSTRAINT oauth_states_one_binding CHECK ((browser_hash IS NULL) <> (app_challenge IS NULL));

CREATE TABLE identity.app_handoffs (
    code_hash     bytea       PRIMARY KEY CHECK (octet_length(code_hash) = 32),
    user_id       uuid        NOT NULL REFERENCES identity.users (id) ON DELETE CASCADE,
    app_challenge bytea       NOT NULL CHECK (octet_length(app_challenge) = 32),
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    CHECK (expires_at > created_at)
);
CREATE INDEX app_handoffs_expires_idx ON identity.app_handoffs (expires_at);
