-- ADR-0037. The student roster holds only active users. A student who comes from a group must also
-- hold the institution role `student`; a direct `student` membership keeps working for a teacher who
-- joined as a student (ADR-0004). `identity.users` (status, role) is declared there as an interface
-- of identity for this view. The columns are as in 0015.
CREATE OR REPLACE VIEW teaching.effective_members (course_id, user_id, role, source) AS
    SELECT m.course_id, m.user_id, m.role, 'direct'::text
    FROM teaching.memberships m
    JOIN identity.users u ON u.id = m.user_id
    WHERE m.role <> 'student' OR u.status = 'active'
  UNION ALL
    SELECT r.course_id, r.user_id, 'student'::text, 'group'::text
    FROM teaching.group_roster r
    JOIN identity.users u ON u.id = r.user_id AND u.status = 'active' AND u.role = 'student'
    WHERE NOT EXISTS (SELECT 1 FROM teaching.memberships m
                      WHERE m.course_id = r.course_id AND m.user_id = r.user_id)
      AND NOT EXISTS (SELECT 1 FROM teaching.course_exclusions e
                      WHERE e.course_id = r.course_id AND e.user_id = r.user_id);

-- A teacher membership of an institution student breaks ADR-0004 and cannot be written any more
-- (ADR-0037). Remove the ones that exist, except an owner's: an owner stays a course teacher, and
-- an admin has to pass such a course on.
DELETE FROM teaching.memberships m
USING identity.users u, teaching.courses c
WHERE m.role = 'teacher' AND u.id = m.user_id AND u.role = 'student'
  AND c.id = m.course_id AND c.owner_id <> m.user_id;
