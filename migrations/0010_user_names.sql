-- ADR-0029. A user has a name and a surname instead of one display name.
-- Existing rows are split at the first run of whitespace: the first word is the name, the rest is
-- the surname. A one-word display name has no surname to split off, so it becomes both parts;
-- nothing is published yet and an admin can correct such a row.
ALTER TABLE identity.users ADD COLUMN name text, ADD COLUMN surname text;

UPDATE identity.users SET
    name    = substring(display_name FROM '^\S+'),
    surname = COALESCE(NULLIF(btrim(substring(display_name FROM '^\S+\s+(.*)$')), ''),
                       substring(display_name FROM '^\S+'));

ALTER TABLE identity.users
    ALTER COLUMN name SET NOT NULL,
    ALTER COLUMN surname SET NOT NULL,
    ADD CONSTRAINT users_name_check
        CHECK (name = btrim(name) AND char_length(name) BETWEEN 1 AND 100),
    ADD CONSTRAINT users_surname_check
        CHECK (surname = btrim(surname) AND char_length(surname) BETWEEN 1 AND 100),
    DROP COLUMN display_name;
