-- ADR-0006. teaching's declared read interface for "who takes part in a course", with the columns
-- (course_id, user_id, role). Other modules read this view and never teaching.memberships. Today it
-- is the direct memberships; linked groups and subgroups extend this one definition later, and
-- every reader that means "members of the course" keeps working unchanged.
CREATE VIEW teaching.effective_members (course_id, user_id, role) AS
    SELECT course_id, user_id, role FROM teaching.memberships;
