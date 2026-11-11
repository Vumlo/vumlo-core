//! Business rules of the plugin reads. Each function checks the scope of the caller first, then the
//! course. A resource that is not there is `NotFound`; one in a course the plugin may not reach is
//! `Forbidden`. A list names a course or covers every course the plugin reaches.

use access::{
    AccessError, CourseRole, PluginCaller, Scope, authorize_plugin, authorize_plugin_course,
    plugin_course_filter,
};
use chrono::{DateTime, Utc};
use common::ApiError;
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::model::{
    Cursor, MailAudience, MailPerson, MailRecipients, MailTarget, PluginAnnouncement,
    PluginAssignment, PluginAttempt, PluginContact, PluginDeletion, PluginFormAnswers, PluginGrade,
    PluginList, PluginMember, PluginPage, PluginQuiz, PluginSubmission, PluginSubmissionRow,
};
use super::repo::{self, After, AssignmentRow};
use crate::content::{AssignmentType, BuiltinType};
use crate::teaching;

const FORM: &str = "form";
const QUIZ: &str = "quiz";

#[derive(Debug, thiserror::Error)]
pub enum PluginReadError {
    #[error(transparent)]
    Forbidden(#[from] AccessError),
    #[error("not found")]
    NotFound,
    /// The audience of a mail is larger than `mail.max_audience`.
    #[error("the audience has more than {max} people")]
    AudienceTooLarge { max: u32 },
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<PluginReadError> for ApiError {
    fn from(error: PluginReadError) -> Self {
        match error {
            PluginReadError::Forbidden(e) => e.into(),
            PluginReadError::NotFound => Self::NotFound,
            // A retry can never succeed, so this is not a `429` (ADR-0085).
            PluginReadError::AudienceTooLarge { max } => Self::invalid_field(
                "to",
                format!(
                    "this audience has more than {max} people; send to a smaller group, or ask the administrator to raise mail.max_audience"
                ),
            ),
            PluginReadError::Db(e) => Self::from_db("learning", &e),
        }
    }
}

fn by_time(at: DateTime<Utc>, id: Uuid) -> Cursor {
    Cursor {
        at,
        a: id,
        b: Uuid::nil(),
    }
}

fn limit(page: &PluginPage) -> i64 {
    i64::from(page.limit) + 1
}

/// The course exists. For a course named in a path, after the check that the plugin may reach it.
async fn course_must_exist(
    conn: &mut PgConnection,
    course_id: Uuid,
) -> Result<(), PluginReadError> {
    if teaching::course_exists(conn, course_id).await? {
        Ok(())
    } else {
        Err(PluginReadError::NotFound)
    }
}

/// The course and the type of an assignment the plugin may reach, once it has `scope`.
async fn reachable_assignment(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    scope: Scope,
    assignment_id: Uuid,
) -> Result<(Uuid, String), PluginReadError> {
    authorize_plugin(caller, scope)?;
    let (course_id, kind) = repo::assignment_header(conn, assignment_id)
        .await?
        .ok_or(PluginReadError::NotFound)?;
    authorize_plugin_course(caller, scope, course_id)?;
    Ok((course_id, kind))
}

fn assignment_of(row: AssignmentRow) -> PluginAssignment {
    let fields = (row.kind == FORM).then(|| match row.config.get("fields") {
        Some(Value::Array(fields)) => fields.clone(),
        _ => Vec::new(),
    });
    PluginAssignment {
        id: row.id,
        course_id: row.course_id,
        kind: row.kind,
        title: row.title,
        due_at: row.due_at,
        max_points: row.max_points,
        fields,
        updated_at: row.updated_at,
    }
}

/// The assignments a student could see, in `course_id` or in every course the plugin reaches,
/// changed at or after `since`.
pub async fn assignments(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    course_id: Option<Uuid>,
    since: Option<DateTime<Utc>>,
    page: PluginPage,
) -> Result<PluginList<PluginAssignment>, PluginReadError> {
    let only = plugin_course_filter(caller, Scope::AssignmentsRead, course_id)?;
    if let Some(course_id) = course_id {
        course_must_exist(conn, course_id).await?;
    }
    let rows = repo::assignments(
        conn,
        only.as_deref(),
        None,
        since,
        &After::from(page.after),
        limit(&page),
    )
    .await?;
    let items = rows.into_iter().map(assignment_of).collect();
    Ok(PluginList::from_rows(items, page.limit, |a| {
        by_time(a.updated_at, a.id)
    }))
}

pub async fn assignment(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    assignment_id: Uuid,
) -> Result<PluginAssignment, PluginReadError> {
    reachable_assignment(conn, caller, Scope::AssignmentsRead, assignment_id).await?;
    let mut rows =
        repo::assignments(conn, None, Some(assignment_id), None, &After::from(None), 1).await?;
    rows.pop()
        .map(assignment_of)
        .ok_or(PluginReadError::NotFound)
}

/// A submitted attempt, never a draft.
pub async fn submission(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    submission_id: Uuid,
) -> Result<PluginSubmission, PluginReadError> {
    authorize_plugin(caller, Scope::SubmissionsRead)?;
    let row = repo::submitted(conn, submission_id)
        .await?
        .ok_or(PluginReadError::NotFound)?;
    authorize_plugin_course(caller, Scope::SubmissionsRead, row.course_id)?;
    Ok(PluginSubmission {
        id: row.id,
        course_id: row.course_id,
        assignment_id: row.assignment_id,
        student_id: row.student_id,
        attempt: row.attempt,
        content: row.content,
        submitted_at: row.submitted_at,
    })
}

/// The submitted attempt a plugin granted `reports:write` wants to report on is there, in a course it
/// may reach. The scope is checked first, so a plugin without it learns nothing about which ids
/// exist.
pub async fn reportable_submission(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    submission_id: Uuid,
) -> Result<(), PluginReadError> {
    authorize_plugin(caller, Scope::ReportsWrite)?;
    let course_id = repo::submission_course(conn, submission_id)
        .await?
        .ok_or(PluginReadError::NotFound)?;
    authorize_plugin_course(caller, Scope::ReportsWrite, course_id)?;
    Ok(())
}

/// The assignment to report on is there, in a course the plugin may reach.
pub async fn reportable_assignment(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    assignment_id: Uuid,
) -> Result<(), PluginReadError> {
    reachable_assignment(conn, caller, Scope::ReportsWrite, assignment_id).await?;
    Ok(())
}

/// The course to report on is there, and the plugin may reach it.
pub async fn reportable_course(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    course_id: Uuid,
) -> Result<(), PluginReadError> {
    authorize_plugin_course(caller, Scope::ReportsWrite, course_id)?;
    course_must_exist(conn, course_id).await
}

/// The values of a submitted `form`, by field id. Any other type is `NotFound`.
pub async fn form_answers(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    submission_id: Uuid,
) -> Result<PluginFormAnswers, PluginReadError> {
    authorize_plugin(caller, Scope::AssignmentsRead)?;
    let row = repo::submitted(conn, submission_id)
        .await?
        .ok_or(PluginReadError::NotFound)?;
    authorize_plugin_course(caller, Scope::AssignmentsRead, row.course_id)?;
    if row.assignment_type != FORM {
        return Err(PluginReadError::NotFound);
    }
    let answers = match row.content.get("answers") {
        Some(Value::Object(answers)) => answers.clone(),
        _ => Default::default(),
    };
    Ok(PluginFormAnswers {
        submission_id: row.id,
        course_id: row.course_id,
        assignment_id: row.assignment_id,
        student_id: row.student_id,
        answers,
    })
}

/// The submitted attempts of an assignment, submitted at or after `since`.
pub async fn submissions(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    assignment_id: Uuid,
    since: Option<DateTime<Utc>>,
    page: PluginPage,
) -> Result<PluginList<PluginSubmissionRow>, PluginReadError> {
    reachable_assignment(conn, caller, Scope::SubmissionsRead, assignment_id).await?;
    let rows = repo::submissions(
        conn,
        assignment_id,
        since,
        &After::from(page.after),
        limit(&page),
    )
    .await?;
    let items = rows
        .into_iter()
        .map(|row| PluginSubmissionRow {
            id: row.id,
            student_id: row.student_id,
            attempt: row.attempt,
            submitted_at: row.submitted_at,
            is_late: row.is_late,
            graded_at: row.graded_at,
        })
        .collect();
    Ok(PluginList::from_rows(items, page.limit, |s| {
        by_time(s.submitted_at, s.id)
    }))
}

/// The people of a course, by user id. A full snapshot: the roster has no change time to filter by.
pub async fn members(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    course_id: Uuid,
    role: Option<CourseRole>,
    page: PluginPage,
) -> Result<PluginList<PluginMember>, PluginReadError> {
    authorize_plugin_course(caller, Scope::RosterRead, course_id)?;
    course_must_exist(conn, course_id).await?;
    let rows = repo::members(
        conn,
        course_id,
        role.map(CourseRole::as_str),
        &After::from(page.after),
        limit(&page),
    )
    .await?;
    Ok(PluginList::from_rows(rows, page.limit, |m| {
        Cursor::by_id(m.user_id)
    }))
}

/// The published grades of a course, of every course the plugin reaches, or of one assignment.
pub async fn grades(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    course_id: Option<Uuid>,
    assignment_id: Option<Uuid>,
    since: Option<DateTime<Utc>>,
    page: PluginPage,
) -> Result<PluginList<PluginGrade>, PluginReadError> {
    let only = plugin_course_filter(caller, Scope::GradesRead, course_id)?;
    if let Some(course_id) = course_id {
        course_must_exist(conn, course_id).await?;
    }
    if let Some(assignment_id) = assignment_id {
        reachable_assignment(conn, caller, Scope::GradesRead, assignment_id).await?;
    }
    let rows = repo::grades(
        conn,
        only.as_deref(),
        assignment_id,
        since,
        &After::from(page.after),
        limit(&page),
    )
    .await?;
    Ok(PluginList::from_rows(rows, page.limit, |g| Cursor {
        at: g.published_at,
        a: g.assignment_id,
        b: g.student_id,
    }))
}

/// The questions of a quiz, without the answer key.
pub async fn quiz(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    assignment_id: Uuid,
) -> Result<PluginQuiz, PluginReadError> {
    let (_, kind) = reachable_assignment(conn, caller, Scope::QuizzesRead, assignment_id).await?;
    if kind != QUIZ {
        return Err(PluginReadError::NotFound);
    }
    let row = repo::assignments(conn, None, Some(assignment_id), None, &After::from(None), 1)
        .await?
        .pop()
        .ok_or(PluginReadError::NotFound)?;
    Ok(PluginQuiz {
        assignment_id: row.id,
        course_id: row.course_id,
        max_points: row.max_points,
        config: BuiltinType::Quiz.student_config(&row.config),
    })
}

/// The submitted attempts at a quiz with their score per question.
pub async fn attempts(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    assignment_id: Uuid,
    since: Option<DateTime<Utc>>,
    page: PluginPage,
) -> Result<PluginList<PluginAttempt>, PluginReadError> {
    let (_, kind) = reachable_assignment(conn, caller, Scope::QuizzesRead, assignment_id).await?;
    if kind != QUIZ {
        return Err(PluginReadError::NotFound);
    }
    let rows = repo::attempts(
        conn,
        assignment_id,
        since,
        &After::from(page.after),
        limit(&page),
    )
    .await?;
    let items = rows
        .into_iter()
        .map(|row| PluginAttempt {
            id: row.id,
            student_id: row.student_id,
            attempt: row.attempt,
            scores: row.scores.unwrap_or_default(),
            started_at: row.started_at,
            submitted_at: row.submitted_at,
        })
        .collect();
    Ok(PluginList::from_rows(items, page.limit, |a| {
        by_time(a.submitted_at, a.id)
    }))
}

/// The email of those of `ids` who are in a course the plugin reaches. The others are left out.
pub async fn contacts(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    ids: &[Uuid],
) -> Result<Vec<PluginContact>, PluginReadError> {
    let only = plugin_course_filter(caller, Scope::UsersContact, None)?;
    Ok(repo::contacts(conn, ids, only.as_deref()).await?)
}

/// The people a plugin's mail is for (scope `mail:send`, ADR-0085). One `user_id` must be on the
/// roster of a course the plugin reaches (`404` for a user nobody knows, `403` for one outside the
/// grant); a `course_id` must be one it reaches. Only active people with a verified address are
/// returned; the rest are counted. More than `max_audience` people is an error and nothing is
/// returned: the roster is read one row past the limit to tell.
pub async fn mail_recipients(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    target: MailTarget,
    max_audience: u32,
) -> Result<MailRecipients, PluginReadError> {
    let rows = match target {
        MailTarget::User(user_id) => {
            let only = plugin_course_filter(caller, Scope::MailSend, None)?;
            let person = repo::mail_person(conn, user_id)
                .await?
                .ok_or(PluginReadError::NotFound)?;
            if !repo::on_roster(conn, user_id, only.as_deref()).await? {
                return Err(AccessError::Denied.into());
            }
            vec![person]
        }
        MailTarget::Course {
            course_id,
            audience,
        } => {
            authorize_plugin_course(caller, Scope::MailSend, course_id)?;
            course_must_exist(conn, course_id).await?;
            let roles: &[&str] = match audience {
                MailAudience::Students => &["student"],
                MailAudience::Teachers => &["teacher", "assistant"],
                MailAudience::All => &["student", "teacher", "assistant"],
            };
            let roles: Vec<String> = roles.iter().map(|role| (*role).to_owned()).collect();
            let rows =
                repo::mail_audience(conn, course_id, &roles, i64::from(max_audience) + 1).await?;
            if rows.len() > max_audience as usize {
                return Err(PluginReadError::AudienceTooLarge { max: max_audience });
            }
            rows
        }
    };
    let mut recipients = MailRecipients {
        people: Vec::new(),
        no_address: 0,
        disabled: 0,
    };
    for row in rows {
        match (row.active, row.verified, row.email) {
            (false, _, _) => recipients.disabled += 1,
            (true, true, Some(email)) => recipients.people.push(MailPerson {
                user_id: row.user_id,
                name: format!("{} {}", row.name, row.surname),
                email,
            }),
            (true, _, _) => recipients.no_address += 1,
        }
    }
    Ok(recipients)
}

/// The announcements of one course or of every course the plugin reaches, changed at or after
/// `since`.
pub async fn announcements(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    course_id: Option<Uuid>,
    since: Option<DateTime<Utc>>,
    page: PluginPage,
) -> Result<PluginList<PluginAnnouncement>, PluginReadError> {
    let only = plugin_course_filter(caller, Scope::AnnouncementsRead, course_id)?;
    if let Some(course_id) = course_id {
        course_must_exist(conn, course_id).await?;
    }
    let rows = repo::announcements(
        conn,
        only.as_deref(),
        since,
        &After::from(page.after),
        limit(&page),
    )
    .await?;
    Ok(PluginList::from_rows(rows, page.limit, |a| {
        by_time(a.updated_at, a.id)
    }))
}

/// What was deleted, of the kind the scope reads (ADR-0081): `assignments:read` for assignments,
/// `announcements:read` for announcements. A course that is itself gone has no row to find, so a
/// `course_id` that names nothing is an empty list and not a `404`; the grant is still checked.
pub async fn deleted(
    conn: &mut PgConnection,
    caller: &PluginCaller,
    what: Deleted,
    course_id: Option<Uuid>,
    since: DateTime<Utc>,
    page: PluginPage,
) -> Result<PluginList<PluginDeletion>, PluginReadError> {
    let (scope, kind) = match what {
        Deleted::Assignments => (Scope::AssignmentsRead, "assignment"),
        Deleted::Announcements => (Scope::AnnouncementsRead, "announcement"),
    };
    let only = plugin_course_filter(caller, scope, course_id)?;
    let rows = repo::deletions(
        conn,
        kind,
        only.as_deref(),
        since,
        &After::from(page.after),
        limit(&page),
    )
    .await?;
    Ok(PluginList::from_rows(rows, page.limit, |d| {
        by_time(d.deleted_at, d.id)
    }))
}

/// What a plugin asks to be told was deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deleted {
    Assignments,
    Announcements,
}
