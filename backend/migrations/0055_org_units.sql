-- ADR-0094. Units of the institution (faculty, department, ...), a tree; a person's role on a unit
-- covers every unit below it, and a course may sit in one unit.

CREATE TABLE org.units (
    id         uuid PRIMARY KEY,
    parent_id  uuid REFERENCES org.units (id),          -- no cascade: a unit with children is not deleted
    name       text NOT NULL CHECK (name = btrim(name) AND char_length(name) BETWEEN 1 AND 100),
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT units_name_key UNIQUE NULLS NOT DISTINCT (parent_id, name)
);
CREATE INDEX units_parent_id_idx ON org.units (parent_id);

CREATE TABLE org.unit_roles (
    unit_id uuid NOT NULL REFERENCES org.units (id) ON DELETE CASCADE,
    user_id uuid NOT NULL REFERENCES auth.users (id),
    role    text NOT NULL CHECK (role IN ('manager', 'viewer')),
    PRIMARY KEY (unit_id, user_id)
);
CREATE INDEX unit_roles_user_id_idx ON org.unit_roles (user_id);

CREATE TABLE org.unit_role_abilities (
    role    text NOT NULL CHECK (role IN ('manager', 'viewer')),
    ability text NOT NULL CHECK (ability IN
            ('view_grades', 'edit_content', 'grade', 'manage_roster', 'manage_courses', 'manage_units')),
    PRIMARY KEY (role, ability)
);

INSERT INTO org.unit_role_abilities (role, ability) VALUES
    ('manager', 'view_grades'),
    ('manager', 'manage_courses'),
    ('manager', 'manage_roster'),
    ('manager', 'manage_units'),
    ('viewer', 'view_grades');

ALTER TABLE teaching.courses ADD COLUMN unit_id uuid REFERENCES org.units (id);   -- no cascade
CREATE INDEX courses_unit_id_idx ON teaching.courses (unit_id);

-- Every unit with itself and each unit below it. `CYCLE` is a backstop: a bad row can never hang a
-- query (a move is checked in the service, under a table lock).
CREATE VIEW org.unit_subtree AS
WITH RECURSIVE tree (root_id, unit_id) AS (
    SELECT id, id FROM org.units
    UNION ALL
    SELECT tree.root_id, child.id
    FROM tree
    JOIN org.units child ON child.parent_id = tree.unit_id
) CYCLE unit_id SET is_cycle USING path
SELECT root_id, unit_id FROM tree WHERE NOT is_cycle;

-- One row per person and per unit their roles cover; `abilities` is the union over every covering
-- role. A grant whose role has no switches on is an empty array: it still means view.
CREATE VIEW org.unit_grants AS
SELECT roles.user_id,
       subtree.unit_id,
       coalesce(array_agg(DISTINCT abilities.ability) FILTER (WHERE abilities.ability IS NOT NULL),
                '{}'::text[]) AS abilities
FROM org.unit_roles roles
JOIN org.unit_subtree subtree ON subtree.root_id = roles.unit_id
LEFT JOIN org.unit_role_abilities abilities ON abilities.role = roles.role
GROUP BY roles.user_id, subtree.unit_id;
