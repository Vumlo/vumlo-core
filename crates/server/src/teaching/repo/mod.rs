//! SQL only. Every function takes the connection it runs on and never begins a transaction.

use chrono::{DateTime, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    Candidate, Course, CourseGroupLink, CourseKind, Exclusion, LinkedGroup, MemberSource,
    Membership, Person, RosterEntry, TemplateSummary,
};
mod admin;
mod cards;
pub use admin::{
    admin_courses, linked_courses_of_group, owned_courses, studied_courses, taught_courses,
};
pub use cards::{
    card_courses, staff_next_deadline, staff_todo_counts, student_next, student_progress,
    unpublished_items,
};

use crate::identity::Role;
use crate::policy::{CourseRole, CourseStatus};

/// A course together with the asking user's membership in it, if any.
pub struct CourseRow {
    pub course: Course,
    pub role: Option<CourseRole>,
}

/// The shape of every course-with-membership query below.
struct RawCourseRow {
    id: Uuid,
    kind: CourseKind,
    title: String,
    status: CourseStatus,
    owner_id: Uuid,
    term_id: Option<Uuid>,
    created_at: DateTime<Utc>,
    member_role: Option<CourseRole>,
}

impl From<RawCourseRow> for CourseRow {
    fn from(raw: RawCourseRow) -> Self {
        Self {
            course: Course {
                id: raw.id,
                kind: raw.kind,
                title: raw.title,
                status: raw.status,
                owner_id: raw.owner_id,
                term_id: raw.term_id,
                created_at: raw.created_at,
            },
            role: raw.member_role,
        }
    }
}

pub async fn set_published(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"UPDATE teaching.courses SET status = 'published', updated_at = now()
           WHERE id = $1
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, term_id, created_at"#,
        course_id
    )
    .fetch_one(conn)
    .await
}

/// Does the user own a course?
pub async fn owns_courses(conn: &mut PgConnection, user_id: Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM teaching.courses WHERE owner_id = $1) AS "owns!""#,
        user_id
    )
    .fetch_one(conn)
    .await
}

/// Deletes every `teacher` membership of the user and returns the courses they were in.
pub async fn delete_teacher_memberships(
    conn: &mut PgConnection,
    user_id: Uuid,
) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar!(
        "DELETE FROM teaching.memberships WHERE user_id = $1 AND role = 'teacher'
         RETURNING course_id",
        user_id
    )
    .fetch_all(conn)
    .await
}

/// Hands the course to a new owner.
pub async fn set_owner(
    conn: &mut PgConnection,
    course_id: Uuid,
    owner_id: Uuid,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"UPDATE teaching.courses SET owner_id = $2, updated_at = now()
           WHERE id = $1
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, term_id, created_at"#,
        course_id,
        owner_id
    )
    .fetch_one(conn)
    .await
}

/// The course and `user_id`'s membership in it.
pub async fn find_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<Option<CourseRow>, sqlx::Error> {
    let row = sqlx::query_as!(
        RawCourseRow,
        r#"SELECT c.id, c.kind AS "kind: CourseKind", c.title,
                  c.status AS "status: CourseStatus", c.owner_id, c.term_id, c.created_at,
                  m.role AS "member_role?: CourseRole"
           FROM teaching.courses c
           LEFT JOIN teaching.effective_members m ON m.course_id = c.id AND m.user_id = $2
           WHERE c.id = $1"#,
        course_id,
        user_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(CourseRow::from))
}

/// Locks the course row until the transaction ends. Writers on one course queue here.
///
/// It returns the course only, never a membership. A statement that waits for a row lock
/// re-reads just the locked row; anything joined to it keeps the statement's earlier snapshot.
/// So the caller reads memberships in a later statement, once it holds the lock, and sees what
/// the writer it waited for committed.
pub async fn lock_course(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Option<Course>, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"SELECT id, kind AS "kind: CourseKind", title,
                  status AS "status: CourseStatus", owner_id, term_id, created_at
           FROM teaching.courses
           WHERE id = $1
           FOR UPDATE"#,
        course_id
    )
    .fetch_optional(conn)
    .await
}

/// Courses `user_id` owns or belongs to, after `after`, in id order, at most `limit`. A student
/// membership only counts once the course is published, and stays after it is archived, the same
/// rule `policy` applies per course (ADR-0048); it lives here too so a page is not emptied by rows the caller may not see.
///
/// It starts from the user's own rows (their places on rosters, the courses they own) and joins
/// the course tables to those, so the cost follows the number of courses the user has, not the
/// number of courses the installation has (audit 014, A-12).
pub async fn list_courses_of(
    conn: &mut PgConnection,
    user_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<CourseRow>, sqlx::Error> {
    let rows = sqlx::query_as!(
        RawCourseRow,
        r#"SELECT c.id, c.kind AS "kind: CourseKind", c.title,
                  c.status AS "status: CourseStatus", c.owner_id, c.term_id, c.created_at,
                  m.role AS "member_role?: CourseRole"
           FROM (SELECT course_id AS id FROM teaching.effective_members WHERE user_id = $1
                 UNION
                 SELECT id FROM teaching.courses WHERE owner_id = $1) mine
           JOIN teaching.courses c ON c.id = mine.id
           LEFT JOIN teaching.effective_members m ON m.course_id = c.id AND m.user_id = $1
           WHERE (c.status IN ('published', 'archived') OR c.owner_id = $1
                  OR m.role IN ('teacher', 'assistant'))
             AND ($2::uuid IS NULL OR c.id > $2)
           ORDER BY c.id
           LIMIT $3"#,
        user_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(CourseRow::from).collect())
}

/// The user's place on the roster of the course, from the view: a direct membership, or a student
/// through a linked group (ADR-0033).
pub async fn find_member(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<Option<RosterEntry>, sqlx::Error> {
    sqlx::query_as!(
        RosterEntry,
        r#"SELECT role AS "role!: CourseRole", source AS "source!: MemberSource"
           FROM teaching.effective_members
           WHERE course_id = $1 AND user_id = $2"#,
        course_id,
        user_id
    )
    .fetch_optional(conn)
    .await
}

/// Adds the member or changes their role; `created_at` is kept on a change.
pub async fn upsert_membership(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
    role: CourseRole,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO teaching.memberships (course_id, user_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (course_id, user_id) DO UPDATE SET role = EXCLUDED.role",
        course_id,
        user_id,
        role.as_str()
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_membership(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM teaching.memberships WHERE course_id = $1 AND user_id = $2",
        course_id,
        user_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// The roster of a course after `after` (a user id), in user-id order, at most `limit`: direct
/// members and students through linked groups.
pub async fn list_members(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Membership>, sqlx::Error> {
    sqlx::query_as!(
        Membership,
        r#"SELECT m.course_id AS "course_id!", m.user_id AS "user_id!",
                  m.role AS "role!: CourseRole", (m.user_id = c.owner_id) AS "is_owner!",
                  m.source AS "source!: MemberSource"
           FROM teaching.effective_members m
           JOIN teaching.courses c ON c.id = m.course_id
           WHERE m.course_id = $1 AND ($2::uuid IS NULL OR m.user_id > $2)
           ORDER BY m.user_id
           LIMIT $3"#,
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

/// Changes the title and/or the term. `set_term` says whether `term_id` was sent (it may be null).
pub async fn update_course(
    conn: &mut PgConnection,
    course_id: Uuid,
    title: Option<&str>,
    set_term: bool,
    term_id: Option<Uuid>,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"UPDATE teaching.courses
           SET title = COALESCE($2, title),
               term_id = CASE WHEN $3 THEN $4 ELSE term_id END,
               updated_at = now()
           WHERE id = $1
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, term_id, created_at"#,
        course_id,
        title,
        set_term,
        term_id
    )
    .fetch_one(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Course info, lifecycle, templates, people and candidates (ADR-0048). The read models join the
// tables of other modules; nothing here writes outside `teaching`.
// ---------------------------------------------------------------------------------------------

/// The part of the course header that `teaching` can read in one statement.
pub struct InfoRow {
    pub code: String,
    pub description: String,
    pub term_name: Option<String>,
    pub owner_name: String,
    pub owner_surname: String,
    pub owner_email: String,
    pub updated_at: DateTime<Utc>,
    pub student_count: i64,
    pub assistant_count: i64,
    /// People off the roster who still have a value or an attempt in the course (ADR-0037).
    pub former_count: i64,
}

pub async fn info_row(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Option<InfoRow>, sqlx::Error> {
    sqlx::query_as!(
        InfoRow,
        r#"SELECT c.code, c.description, t.name AS "term_name?",
                  u.name AS owner_name, u.surname AS owner_surname, u.email AS owner_email,
                  c.updated_at,
                  (SELECT count(*) FROM teaching.effective_members m
                    WHERE m.course_id = c.id AND m.role = 'student') AS "student_count!",
                  (SELECT count(*) FROM teaching.effective_members m
                    WHERE m.course_id = c.id AND m.role = 'assistant') AS "assistant_count!",
                  (SELECT count(*) FROM identity.users f WHERE f.id IN (
                       SELECT g.student_id FROM gradebook.grades g
                       JOIN content.items i ON i.id = g.assignment_id
                       WHERE i.course_id = c.id
                         AND (g.draft_score IS NOT NULL OR g.draft_feedback IS NOT NULL
                              OR g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
                       UNION
                       SELECT s.student_id FROM assessment.submissions s
                       JOIN content.items i ON i.id = s.assignment_id
                       WHERE i.course_id = c.id AND s.status = 'submitted')
                     AND NOT EXISTS (SELECT 1 FROM teaching.effective_members m
                                     WHERE m.course_id = c.id AND m.user_id = f.id))
                      AS "former_count!"
           FROM teaching.courses c
           JOIN identity.users u ON u.id = c.owner_id
           LEFT JOIN org.terms t ON t.id = c.term_id
           WHERE c.id = $1"#,
        course_id
    )
    .fetch_optional(conn)
    .await
}

/// Each field is kept when `None`.
pub async fn update_info(
    conn: &mut PgConnection,
    course_id: Uuid,
    code: Option<&str>,
    description: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE teaching.courses
         SET code = COALESCE($2, code), description = COALESCE($3, description), updated_at = now()
         WHERE id = $1",
        course_id,
        code,
        description
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn set_status(
    conn: &mut PgConnection,
    course_id: Uuid,
    status: CourseStatus,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"UPDATE teaching.courses SET status = $2, updated_at = now()
           WHERE id = $1
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, term_id, created_at"#,
        course_id,
        status.as_str()
    )
    .fetch_one(conn)
    .await
}

/// The memberships, links and exclusions go with the row (`ON DELETE CASCADE`), and so does the
/// content tree. Returns how many rows were deleted.
pub async fn delete_course(conn: &mut PgConnection, course_id: Uuid) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM teaching.courses WHERE id = $1", course_id)
        .execute(conn)
        .await?;
    Ok(result.rows_affected())
}

/// The row of a course that is the copy of another one.
pub struct CourseCopy<'a> {
    pub id: Uuid,
    pub kind: CourseKind,
    pub title: &'a str,
    pub owner_id: Uuid,
    pub term_id: Option<Uuid>,
    pub code: &'a str,
    pub description: &'a str,
}

/// Inserts a course of `copy.kind` with a code and a description.
pub async fn insert_copy(
    conn: &mut PgConnection,
    copy: CourseCopy<'_>,
) -> Result<Course, sqlx::Error> {
    sqlx::query_as!(
        Course,
        r#"INSERT INTO teaching.courses
               (id, kind, title, owner_id, created_by, term_id, code, description)
           VALUES ($1, $2, $3, $4, $4, $5, $6, $7)
           RETURNING id, kind AS "kind: CourseKind", title,
                     status AS "status: CourseStatus", owner_id, term_id, created_at"#,
        copy.id,
        copy.kind.as_str(),
        copy.title,
        copy.owner_id,
        copy.term_id,
        copy.code,
        copy.description
    )
    .fetch_one(conn)
    .await
}

/// The code and the description of a course, for its copy.
pub async fn code_and_description(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<(String, String), sqlx::Error> {
    let row = sqlx::query!(
        "SELECT code, description FROM teaching.courses WHERE id = $1",
        course_id
    )
    .fetch_one(conn)
    .await?;
    Ok((row.code, row.description))
}

/// The templates the user owns or teaches, after `after`, in id order, at most `limit`.
pub async fn list_templates(
    conn: &mut PgConnection,
    user_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<TemplateSummary>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT c.id, c.title, c.status AS "status: CourseStatus", c.owner_id, c.term_id,
                  c.created_at, c.updated_at,
                  (SELECT count(*) FROM content.modules WHERE course_id = c.id) AS "sections!",
                  (SELECT count(*) FROM content.items
                    WHERE course_id = c.id AND kind = 'text') AS "lectures!",
                  (SELECT count(*) FROM content.items
                    WHERE course_id = c.id AND kind = 'assignment') AS "assignments!"
           FROM teaching.courses c
           WHERE c.kind = 'template'
             AND (c.owner_id = $1
                  OR EXISTS (SELECT 1 FROM teaching.memberships m
                             WHERE m.course_id = c.id AND m.user_id = $1
                               AND m.role IN ('teacher', 'assistant')))
             AND ($2::uuid IS NULL OR c.id > $2)
           ORDER BY c.id
           LIMIT $3"#,
        user_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| TemplateSummary {
            course: Course {
                id: row.id,
                kind: CourseKind::Template,
                title: row.title,
                status: row.status,
                owner_id: row.owner_id,
                term_id: row.term_id,
                created_at: row.created_at,
            },
            sections: row.sections,
            lectures: row.lectures,
            assignments: row.assignments,
            updated_at: row.updated_at,
        })
        .collect())
}

/// Everyone on the roster, staff first and then by surname, with the group a student comes from.
/// Emails are selected only `with_emails`: a student's query never reads them (ADR-0048).
pub async fn list_people(
    conn: &mut PgConnection,
    course_id: Uuid,
    with_emails: bool,
) -> Result<Vec<Person>, sqlx::Error> {
    sqlx::query_as!(
        Person,
        r#"SELECT u.id AS user_id, u.name, u.surname,
                  CASE WHEN $2 THEN u.email END AS "email?",
                  m.role AS "role!: CourseRole", (u.id = c.owner_id) AS "is_owner!",
                  m.source AS "source!: MemberSource",
                  CASE WHEN m.source = 'group' THEN
                      (SELECT string_agg(s.label, ', ' ORDER BY s.label) FROM (
                           SELECT DISTINCT CASE WHEN sg.id IS NULL THEN g.name
                                                ELSE g.name || ' / ' || sg.name END AS label
                           FROM teaching.group_roster_links l
                           JOIN teaching.course_groups cg ON cg.id = l.link_id
                           JOIN org.groups g ON g.id = cg.group_id
                           LEFT JOIN org.subgroups sg ON sg.id = cg.subgroup_id
                           WHERE l.course_id = m.course_id AND l.user_id = m.user_id) s)
                  END AS "group?",
                  false AS "former!"
           FROM teaching.effective_members m
           JOIN teaching.courses c ON c.id = m.course_id
           JOIN identity.users u ON u.id = m.user_id
           WHERE m.course_id = $1
           ORDER BY CASE m.role WHEN 'teacher' THEN 0 WHEN 'assistant' THEN 1 ELSE 2 END,
                    u.surname, u.name, u.id"#,
        course_id,
        with_emails
    )
    .fetch_all(conn)
    .await
}

/// People off the roster who still have a value or an attempt in the course (ADR-0037).
pub async fn list_former(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<Person>, sqlx::Error> {
    sqlx::query_as!(
        Person,
        r#"SELECT u.id AS user_id, u.name, u.surname, u.email AS "email?",
                  'student' AS "role!: CourseRole", false AS "is_owner!",
                  'direct' AS "source!: MemberSource", NULL::text AS "group?",
                  true AS "former!"
           FROM identity.users u
           WHERE u.id IN (
                   SELECT g.student_id FROM gradebook.grades g
                   JOIN content.items i ON i.id = g.assignment_id
                   WHERE i.course_id = $1
                     AND (g.draft_score IS NOT NULL OR g.draft_feedback IS NOT NULL
                          OR g.published_score IS NOT NULL OR g.published_feedback IS NOT NULL)
                   UNION
                   SELECT s.student_id FROM assessment.submissions s
                   JOIN content.items i ON i.id = s.assignment_id
                   WHERE i.course_id = $1 AND s.status = 'submitted')
             AND NOT EXISTS (SELECT 1 FROM teaching.effective_members m
                             WHERE m.course_id = $1 AND m.user_id = u.id)
           ORDER BY u.surname, u.name, u.id"#,
        course_id
    )
    .fetch_all(conn)
    .await
}

/// The links of a course with the students each one brings: people on the roster through a group
/// (not excluded, not a direct member).
pub async fn list_link_summaries(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<Vec<LinkedGroup>, sqlx::Error> {
    sqlx::query_as!(
        LinkedGroup,
        r#"SELECT cg.id AS link_id, g.name AS group_name, sg.name AS "subgroup_name?",
                  (SELECT count(*) FROM teaching.group_roster_links l
                   JOIN teaching.effective_members m
                     ON m.course_id = l.course_id AND m.user_id = l.user_id AND m.source = 'group'
                   WHERE l.link_id = cg.id) AS "student_count!",
                  cg.created_at AS linked_at
           FROM teaching.course_groups cg
           JOIN org.groups g ON g.id = cg.group_id
           LEFT JOIN org.subgroups sg ON sg.id = cg.subgroup_id
           WHERE cg.course_id = $1
           ORDER BY g.name, sg.name NULLS FIRST, cg.id"#,
        course_id
    )
    .fetch_all(conn)
    .await
}

/// Active users who are not on the course, whose name, surname, "name surname" or email contains
/// the already escaped `pattern` (`%…%`). Admins cannot hold a course role, so they are left out.
pub async fn search_candidates(
    conn: &mut PgConnection,
    course_id: Uuid,
    pattern: &str,
    limit: i64,
) -> Result<Vec<Candidate>, sqlx::Error> {
    sqlx::query_as!(
        Candidate,
        r#"SELECT u.id AS user_id, u.name, u.surname, u.email, u.role AS "role: Role"
           FROM identity.users u
           WHERE u.status = 'active' AND u.role <> 'admin'
             AND NOT EXISTS (SELECT 1 FROM teaching.effective_members m
                             WHERE m.course_id = $1 AND m.user_id = u.id)
             AND u.id <> (SELECT owner_id FROM teaching.courses WHERE id = $1)
             AND (u.name ILIKE $2 ESCAPE '\' OR u.surname ILIKE $2 ESCAPE '\'
                  OR u.email ILIKE $2 ESCAPE '\'
                  OR (u.name || ' ' || u.surname) ILIKE $2 ESCAPE '\')
           ORDER BY u.surname, u.name, u.id
           LIMIT $3"#,
        course_id,
        pattern,
        limit
    )
    .fetch_all(conn)
    .await
}

// ---------------------------------------------------------------------------------------------
// Group links and exclusions (ADR-0033)
// ---------------------------------------------------------------------------------------------

/// Links the course to the group or subgroup. `None` when that link exists already.
pub async fn insert_link(
    conn: &mut PgConnection,
    id: Uuid,
    course_id: Uuid,
    group_id: Uuid,
    subgroup_id: Option<Uuid>,
    created_by: Uuid,
) -> Result<Option<CourseGroupLink>, sqlx::Error> {
    sqlx::query_as!(
        CourseGroupLink,
        "INSERT INTO teaching.course_groups (id, course_id, group_id, subgroup_id, created_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING
         RETURNING id, course_id, group_id, subgroup_id, created_at",
        id,
        course_id,
        group_id,
        subgroup_id,
        created_by
    )
    .fetch_optional(conn)
    .await
}

pub async fn find_link_by_target(
    conn: &mut PgConnection,
    course_id: Uuid,
    group_id: Uuid,
    subgroup_id: Option<Uuid>,
) -> Result<Option<CourseGroupLink>, sqlx::Error> {
    sqlx::query_as!(
        CourseGroupLink,
        "SELECT id, course_id, group_id, subgroup_id, created_at
         FROM teaching.course_groups
         WHERE course_id = $1 AND group_id = $2 AND subgroup_id IS NOT DISTINCT FROM $3",
        course_id,
        group_id,
        subgroup_id
    )
    .fetch_optional(conn)
    .await
}

pub async fn list_links(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<CourseGroupLink>, sqlx::Error> {
    sqlx::query_as!(
        CourseGroupLink,
        "SELECT id, course_id, group_id, subgroup_id, created_at
         FROM teaching.course_groups
         WHERE course_id = $1 AND ($2::uuid IS NULL OR id > $2)
         ORDER BY id LIMIT $3",
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

pub async fn delete_link(
    conn: &mut PgConnection,
    course_id: Uuid,
    link_id: Uuid,
) -> Result<Option<CourseGroupLink>, sqlx::Error> {
    sqlx::query_as!(
        CourseGroupLink,
        "DELETE FROM teaching.course_groups WHERE course_id = $1 AND id = $2
         RETURNING id, course_id, group_id, subgroup_id, created_at",
        course_id,
        link_id
    )
    .fetch_optional(conn)
    .await
}

/// Records that the user is off the roster of this course. Keeps an existing exclusion as it is.
pub async fn insert_exclusion(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
    created_by: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO teaching.course_exclusions (course_id, user_id, created_by)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        course_id,
        user_id,
        created_by
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn delete_exclusion(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM teaching.course_exclusions WHERE course_id = $1 AND user_id = $2",
        course_id,
        user_id
    )
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// The exclusions of a course after `after` (a user id), in user-id order, at most `limit`.
pub async fn list_exclusions(
    conn: &mut PgConnection,
    course_id: Uuid,
    after: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Exclusion>, sqlx::Error> {
    sqlx::query_as!(
        Exclusion,
        "SELECT user_id, created_by, created_at FROM teaching.course_exclusions
         WHERE course_id = $1 AND ($2::uuid IS NULL OR user_id > $2)
         ORDER BY user_id LIMIT $3",
        course_id,
        after,
        limit
    )
    .fetch_all(conn)
    .await
}

/// Would a linked group put the user on the roster, ignoring a direct membership and exclusions?
pub async fn in_group_roster(
    conn: &mut PgConnection,
    course_id: Uuid,
    user_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM teaching.group_roster
                          WHERE course_id = $1 AND user_id = $2) AS "found!""#,
        course_id,
        user_id
    )
    .fetch_one(conn)
    .await
}
