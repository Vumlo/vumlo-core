-- ADR-0048. Which link brings which student: the people of a course show a student's group, and a
-- link shows how many students it brings. `group_roster` (0015) collapses the links of a student into
-- one row, so this view keeps one row per link. Like `group_roster`, it ignores direct memberships and
-- exclusions; `effective_members` decides who is really on the roster.
CREATE VIEW teaching.group_roster_links (course_id, user_id, link_id) AS
    SELECT cg.course_id, gm.user_id, cg.id
    FROM teaching.course_groups cg
    JOIN org.group_members gm ON gm.group_id = cg.group_id
    WHERE cg.subgroup_id IS NULL
  UNION ALL
    SELECT cg.course_id, sm.user_id, cg.id
    FROM teaching.course_groups cg
    JOIN org.subgroup_members sm ON sm.subgroup_id = cg.subgroup_id;
