-- ADR-0033. Group links, course exclusions and the live roster.

-- A live link of a course to a whole group (subgroup_id is null) or to one of its subgroups.
-- The group and the subgroup are protected: a group or subgroup that a course uses cannot be deleted.
CREATE TABLE teaching.course_groups (
    id          uuid        PRIMARY KEY,
    course_id   uuid        NOT NULL REFERENCES teaching.courses (id) ON DELETE CASCADE,
    group_id    uuid        NOT NULL REFERENCES org.groups (id) ON DELETE RESTRICT,
    subgroup_id uuid,
    created_by  uuid        NOT NULL REFERENCES identity.users (id),
    created_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT course_groups_target_key UNIQUE NULLS NOT DISTINCT (course_id, group_id, subgroup_id),
    -- the subgroup belongs to the group; not checked when subgroup_id is null
    CONSTRAINT course_groups_subgroup_fkey FOREIGN KEY (subgroup_id, group_id)
        REFERENCES org.subgroups (id, group_id) ON DELETE RESTRICT
);
CREATE INDEX course_groups_group_idx ON teaching.course_groups (group_id);
CREATE INDEX course_groups_subgroup_idx ON teaching.course_groups (subgroup_id) WHERE subgroup_id IS NOT NULL;

-- A person taken off the roster of one course although a linked group would put them on it.
CREATE TABLE teaching.course_exclusions (
    course_id  uuid        NOT NULL REFERENCES teaching.courses (id) ON DELETE CASCADE,
    user_id    uuid        NOT NULL REFERENCES identity.users (id),
    created_by uuid        NOT NULL REFERENCES identity.users (id),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (course_id, user_id)
);
CREATE INDEX course_exclusions_user_idx ON teaching.course_exclusions (user_id, course_id);

-- "My courses" asks the view by user; org.group_members already has (user_id, group_id) from 0014.
CREATE INDEX subgroup_members_user_idx ON org.subgroup_members (user_id, subgroup_id);

-- The students a course gets through its links, ignoring direct memberships and exclusions.
-- UNION (not ALL): a user reachable through several links is one row.
CREATE VIEW teaching.group_roster (course_id, user_id) AS
    SELECT cg.course_id, gm.user_id
    FROM teaching.course_groups cg
    JOIN org.group_members gm ON gm.group_id = cg.group_id
    WHERE cg.subgroup_id IS NULL
  UNION
    SELECT cg.course_id, sm.user_id
    FROM teaching.course_groups cg
    JOIN org.subgroup_members sm ON sm.subgroup_id = cg.subgroup_id;

-- Who takes part in a course: one row per (course, user). Direct membership first, then an
-- exclusion, then a linked group. `source` is appended; the first three columns are as in 0012.
CREATE OR REPLACE VIEW teaching.effective_members (course_id, user_id, role, source) AS
    SELECT m.course_id, m.user_id, m.role, 'direct'::text
    FROM teaching.memberships m
  UNION ALL
    SELECT r.course_id, r.user_id, 'student'::text, 'group'::text
    FROM teaching.group_roster r
    WHERE NOT EXISTS (SELECT 1 FROM teaching.memberships m
                      WHERE m.course_id = r.course_id AND m.user_id = r.user_id)
      AND NOT EXISTS (SELECT 1 FROM teaching.course_exclusions e
                      WHERE e.course_id = r.course_id AND e.user_id = r.user_id);
