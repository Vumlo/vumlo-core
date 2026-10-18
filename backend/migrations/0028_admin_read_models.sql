-- ADR-0053. When a person last signed in, and when they were put in a group. Neither is known for
-- what exists already.
--
-- The sign-in time is a row of its own and not a column of `identity.users`: a login holds the user
-- row `FOR SHARE` (ADR-0038), and two logins of one person that both tried to update that row would
-- wait for each other.
CREATE TABLE identity.last_logins (
    user_id uuid        PRIMARY KEY REFERENCES identity.users (id) ON DELETE CASCADE,
    at      timestamptz NOT NULL
);

ALTER TABLE org.group_members ADD COLUMN added_at timestamptz;
ALTER TABLE org.group_members ALTER COLUMN added_at SET DEFAULT now();
