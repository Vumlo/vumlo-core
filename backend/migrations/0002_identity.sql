CREATE TABLE identity.users (
    id                   uuid        PRIMARY KEY,
    email                text        NOT NULL UNIQUE
                                     CHECK (email = lower(btrim(email)) AND email <> ''),
    display_name         text        NOT NULL
                                     CHECK (display_name = btrim(display_name)
                                            AND char_length(display_name) BETWEEN 1 AND 100),
    role                 text        NOT NULL CHECK (role IN ('admin', 'teacher', 'student')),
    status               text        NOT NULL DEFAULT 'active'
                                     CHECK (status IN ('active', 'disabled')),
    email_verified_at    timestamptz,
    must_change_password boolean     NOT NULL DEFAULT false,
    created_at           timestamptz NOT NULL DEFAULT now(),
    updated_at           timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE identity.identities (
    id            uuid        PRIMARY KEY,
    user_id       uuid        NOT NULL REFERENCES identity.users (id) ON DELETE CASCADE,
    provider      text        NOT NULL CHECK (provider IN ('password', 'google')),
    subject       text        NOT NULL,
    password_hash text,
    created_at    timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider, subject),
    CHECK ((provider = 'password') = (password_hash IS NOT NULL))
);
CREATE INDEX identities_user_id_idx ON identity.identities (user_id);

CREATE TABLE identity.sessions (
    id           uuid        PRIMARY KEY,
    user_id      uuid        NOT NULL REFERENCES identity.users (id) ON DELETE CASCADE,
    token_hash   bytea       NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    kind         text        NOT NULL CHECK (kind IN ('web', 'app', 'cli', 'pat')),
    name         text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    expires_at   timestamptz NOT NULL
);
CREATE INDEX sessions_user_id_idx ON identity.sessions (user_id);
