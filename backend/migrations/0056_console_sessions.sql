-- The door a session came through (ADR-0095). `app` is the student app and every bearer token;
-- `console` is the admin console. Existing sessions stay valid as app sessions.
ALTER TABLE auth.sessions
    ADD COLUMN audience text NOT NULL DEFAULT 'app' CHECK (audience IN ('app', 'console')),
    -- Only a browser session can be a console session: an app client never is.
    ADD CONSTRAINT sessions_console_is_web CHECK (audience = 'app' OR kind = 'web');
