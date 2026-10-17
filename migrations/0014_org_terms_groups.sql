-- ADR-0032. Terms, groups (cohorts), subgroups and their members.
CREATE TABLE org.terms (
    id        uuid PRIMARY KEY,
    name      text NOT NULL CONSTRAINT terms_name_key UNIQUE
              CHECK (name = btrim(name) AND char_length(name) BETWEEN 1 AND 100),
    starts_on date NOT NULL,
    ends_on   date NOT NULL,
    CONSTRAINT terms_dates_check CHECK (ends_on >= starts_on)
);

-- Announced in 0004. A term that a course uses cannot be deleted; the index serves that check.
ALTER TABLE teaching.courses
    ADD CONSTRAINT courses_term_id_fkey FOREIGN KEY (term_id) REFERENCES org.terms (id) ON DELETE RESTRICT;
CREATE INDEX courses_term_id_idx ON teaching.courses (term_id);

CREATE TABLE org.groups (
    id         uuid        PRIMARY KEY,
    name       text        NOT NULL CONSTRAINT groups_name_key UNIQUE
               CHECK (name = btrim(name) AND char_length(name) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE org.group_members (
    group_id uuid NOT NULL REFERENCES org.groups (id) ON DELETE CASCADE,
    user_id  uuid NOT NULL REFERENCES identity.users (id),
    PRIMARY KEY (group_id, user_id)
);
CREATE INDEX group_members_user_id_idx ON org.group_members (user_id, group_id);

CREATE TABLE org.subgroups (
    id       uuid PRIMARY KEY,
    group_id uuid NOT NULL REFERENCES org.groups (id) ON DELETE CASCADE,
    name     text NOT NULL CHECK (name = btrim(name) AND char_length(name) BETWEEN 1 AND 100),
    CONSTRAINT subgroups_name_key UNIQUE (group_id, name),
    UNIQUE (id, group_id)
);

-- A subgroup member is a member of the group: removing the user from the group removes them here.
CREATE TABLE org.subgroup_members (
    subgroup_id uuid NOT NULL,
    group_id    uuid NOT NULL,
    user_id     uuid NOT NULL,
    PRIMARY KEY (subgroup_id, user_id),
    FOREIGN KEY (subgroup_id, group_id) REFERENCES org.subgroups (id, group_id) ON DELETE CASCADE,
    FOREIGN KEY (group_id, user_id) REFERENCES org.group_members (group_id, user_id) ON DELETE CASCADE
);
CREATE INDEX subgroup_members_group_user_idx ON org.subgroup_members (group_id, user_id);
