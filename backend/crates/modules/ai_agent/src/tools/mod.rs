//! What the Buddy can do (ADR-0070). Every tool calls an existing service function as the user's
//! `Actor`, so it sees what that user sees and nothing more; the service authorizes.
//!
//! A *read* tool runs at once and its result is written to JSON field by field, never by serializing
//! a domain type: what the model may read is exactly what is named here (a student's tools name no
//! `draft_*` field, ADR-0028). A *write* tool never runs from the agent loop: [`prepare`] checks it
//! and turns it into a proposal, and [`execute`] runs it when the user confirms.

mod admin;
mod authoring;
mod grading;
mod submitting;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use access::Actor;
use activity::EffectiveProfiles;
use auth::HashedPassword;
use chrono::{DateTime, Utc};
use common::{ApiError, PageParams};
use content::ItemRules;
use gradebook::ColumnStatus;
use learning::items;
use module_api::Institution;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection};
use uuid::Uuid;

use crate::llm::ToolSpec;
use crate::persona::Persona;

use grading::explain;
use submitting::text_of;

/// Rows of one list sent to the model; the rest is counted, not sent.
const MAX_ROWS: usize = 50;

/// The tools offered to this kind of user.
pub fn specs(persona: Persona) -> Vec<ToolSpec> {
    let none = json!({"type": "object", "properties": {}});
    match persona {
        Persona::Student => {
            let mut tools = vec![
                ToolSpec::new(
                    "my_work",
                    "The student's courses and the assignments they can open, with due dates, status \
                 and published scores.",
                    none,
                ),
                ToolSpec::new(
                    "my_grades",
                    "The student's published grades and feedback in one course.",
                    json!({
                        "type": "object",
                        "properties": {"course_id": {"type": "string", "description": "A course id from my_work"}},
                        "required": ["course_id"]
                    }),
                ),
                ToolSpec::new(
                    "assignment_info",
                    "One assignment the student can open: its type, deadline, points and, for a text \
                 assignment, its instructions.",
                    json!({
                        "type": "object",
                        "properties": {"assignment_id": {"type": "string", "description": "An assignment id from my_work"}},
                        "required": ["assignment_id"]
                    }),
                ),
            ];
            tools.extend(submitting::specs());
            tools
        }
        Persona::Teacher | Persona::Admin => {
            let mut tools = vec![
                ToolSpec::new(
                    "teaching_overview",
                    "What the user still has to do in the courses they teach: submissions waiting for \
                     a grade, grades not yet published, upcoming deadlines and unpublished content.",
                    none,
                ),
                ToolSpec::new(
                    "course_gradebook",
                    "The assignments of one course with their ids, and its students with their ids and \
                     their draft and published scores. Use it to find the ids a change needs.",
                    json!({
                        "type": "object",
                        "properties": {"course_id": {"type": "string", "description": "A course id from teaching_overview"}},
                        "required": ["course_id"]
                    }),
                ),
            ];
            tools.push(ToolSpec::new(
                "course_content",
                "The modules of one course with their ids, and the items in them with their ids, \
                 titles, kinds and whether they are published. Use it to find where to put new content.",
                json!({
                    "type": "object",
                    "properties": {"course_id": {"type": "string", "description": "A course id from teaching_overview"}},
                    "required": ["course_id"]
                }),
            ));
            tools.extend(grading::specs());
            tools.extend(authoring::specs());
            if persona == Persona::Admin {
                tools.extend(admin::specs());
            }
            tools
        }
    }
}

/// Whether `name` is a tool that changes something, so it is proposed and not run.
pub fn is_write(name: &str) -> bool {
    grading::NAMES.contains(&name)
        || authoring::NAMES.contains(&name)
        || submitting::NAMES.contains(&name)
        || admin::NAMES.contains(&name)
}

/// The rules the core applies when it writes content, from what the institution configured. `None`
/// only if the editor configuration does not resolve, which the core refuses at start-up.
pub fn rules(institution: &Institution) -> Option<ItemRules<'static>> {
    static PROFILES: OnceLock<Option<EffectiveProfiles>> = OnceLock::new();
    let profiles = PROFILES
        .get_or_init(|| EffectiveProfiles::resolve(&institution.editor).ok())
        .as_ref()?;
    Some(ItemRules {
        scale: institution.grading_scale,
        profiles,
    })
}

/// A change the user may confirm: one of the packs'.
#[derive(Debug)]
enum Change {
    Grading(grading::Write),
    Authoring(authoring::Write),
    Submitting(submitting::Write),
    Admin(admin::Write),
}

impl Change {
    fn parse(name: &str, arguments: &str) -> Result<Self, String> {
        if grading::NAMES.contains(&name) {
            grading::Write::parse(name, arguments).map(Self::Grading)
        } else if authoring::NAMES.contains(&name) {
            authoring::Write::parse(name, arguments).map(Self::Authoring)
        } else if submitting::NAMES.contains(&name) {
            submitting::Write::parse(name, arguments).map(Self::Submitting)
        } else if admin::NAMES.contains(&name) {
            admin::Write::parse(name, arguments).map(Self::Admin)
        } else {
            Err("unknown tool".to_owned())
        }
    }

    /// A change as [`Self::to_json`] stored it, found by its `tool` tag.
    fn from_json(json: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(json).ok()?;
        let name = value.get("tool")?.as_str()?;
        if grading::NAMES.contains(&name) {
            serde_json::from_value(value).ok().map(Self::Grading)
        } else if authoring::NAMES.contains(&name) {
            serde_json::from_value(value).ok().map(Self::Authoring)
        } else if submitting::NAMES.contains(&name) {
            serde_json::from_value(value).ok().map(Self::Submitting)
        } else if admin::NAMES.contains(&name) {
            serde_json::from_value(value).ok().map(Self::Admin)
        } else {
            None
        }
    }

    fn to_json(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Grading(w) => serde_json::to_string(w),
            Self::Authoring(w) => serde_json::to_string(w),
            Self::Submitting(w) => serde_json::to_string(w),
            Self::Admin(w) => serde_json::to_string(w),
        }
    }

    /// What only the student's own chat can vouch for (ADR-0070): see [`submitting::quoted`].
    fn check_words(&self, own_words: &[String]) -> Result<(), String> {
        match self {
            Self::Submitting(w) => w.check_words(own_words),
            _ => Ok(()),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Grading(w) => w.name(),
            Self::Authoring(w) => w.name(),
            Self::Submitting(w) => w.name(),
            Self::Admin(w) => w.name(),
        }
    }

    /// Whether the confirm needs a temporary password made for it (see [`admin`]).
    fn wants_password(&self) -> bool {
        matches!(self, Self::Admin(w) if w.wants_password())
    }

    async fn describe(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
    ) -> Result<BTreeMap<&'static str, String>, String> {
        match self {
            Self::Grading(w) => w.describe(conn, actor).await,
            Self::Authoring(w) => w.describe(conn, actor).await,
            Self::Submitting(w) => w.describe(conn, actor).await,
            Self::Admin(w) => w.describe(conn, actor).await,
        }
    }

    async fn run(
        &self,
        conn: &mut PgConnection,
        actor: &Actor,
        rules: ItemRules<'_>,
        password: Option<&HashedPassword>,
    ) -> Result<(), ApiError> {
        match self {
            Self::Grading(w) => w.run(conn, actor).await,
            Self::Authoring(w) => w.run(conn, actor, rules).await,
            Self::Submitting(w) => w.run(conn, actor, rules).await,
            Self::Admin(w) => w.run(conn, actor, password).await,
        }
    }
}

/// A change the user may confirm: what runs, and what the card shows.
#[derive(Debug)]
pub struct Prepared {
    pub tool: &'static str,
    /// JSON of the checked change, which [`execute`] parses again.
    pub args: String,
    /// JSON object of strings read from the service: titles, names, values.
    pub facts: String,
}

/// Checks a write tool the model asked for and prepares its proposal. The change is parsed
/// strictly, described from what the user may see, and then **run in a transaction that is always
/// rolled back**, so authorization, validation and limits are the service's own and cannot drift
/// from what the confirm will do. Nothing is kept; an `Err` is the refusal for the model.
pub async fn prepare(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: Option<ItemRules<'_>>,
    own_words: &[String],
    persona: Persona,
    name: &str,
    arguments: &str,
) -> Result<Prepared, String> {
    if !specs(persona).iter().any(|s| s.name() == name) {
        return Err("unknown tool".to_owned());
    }
    let rules = rules.ok_or("the editor configuration is not valid, so nothing can be created")?;
    let change = Change::parse(name, arguments)?;
    change.check_words(own_words)?;
    // The dry run of a password reset needs some hash; it is rolled back with the rest.
    let probe = if change.wants_password() {
        let unused = auth::TemporaryPassword::generate();
        let hash = auth::hash_password(unused.expose()).await;
        Some(hash.map_err(|_| "could not prepare that right now".to_owned())?)
    } else {
        None
    };
    let busy = |_| "the database is busy, try again".to_owned();
    let mut dry = conn.begin().await.map_err(busy)?;
    let described = change.describe(&mut dry, actor).await;
    let tried = match described {
        Ok(_) => change
            .run(&mut dry, actor, rules, probe.as_ref())
            .await
            .map_err(|e| explain(&e)),
        Err(_) => Ok(()),
    };
    dry.rollback().await.map_err(busy)?;
    let facts = described?;
    tried?;
    Ok(Prepared {
        tool: change.name(),
        args: change
            .to_json()
            .map_err(|_| "could not store the change".to_owned())?,
        facts: serde_json::to_string(&facts)
            .map_err(|_| "could not store the change".to_owned())?,
    })
}

/// Whether the stored change `args` needs a temporary password to be made before its transaction.
pub fn wants_password(args: &str) -> bool {
    Change::from_json(args).is_some_and(|c| c.wants_password())
}

/// Runs a confirmed change as `actor`, in the caller's transaction. `args` is what [`prepare`]
/// stored; `password` is the hash for the change that [`wants_password`].
pub async fn execute(
    conn: &mut PgConnection,
    actor: &Actor,
    rules: Option<ItemRules<'_>>,
    args: &str,
    password: Option<&HashedPassword>,
) -> Result<(), ApiError> {
    let (Some(change), Some(rules)) = (Change::from_json(args), rules) else {
        tracing::error!("ai_agent: a stored proposal or the editor configuration cannot be read");
        return Err(ApiError::Internal);
    };
    change.run(conn, actor, rules, password).await
}

/// Runs one tool the model asked for and returns what the model reads: JSON, or `{"error": ...}`.
/// A tool the persona was not offered does not run.
pub async fn run(
    conn: &mut PgConnection,
    actor: &Actor,
    persona: Persona,
    name: &str,
    arguments: &str,
    now: DateTime<Utc>,
) -> String {
    if !specs(persona).iter().any(|s| s.name() == name) {
        return error("unknown tool");
    }
    let result = match name {
        "my_work" => my_work(conn, actor, now).await,
        "my_grades" => my_grades(conn, actor, arguments).await,
        "teaching_overview" => teaching_overview(conn, actor, now).await,
        "course_gradebook" => course_gradebook(conn, actor, arguments).await,
        "course_content" => course_content(conn, actor, arguments).await,
        "assignment_info" => assignment_info(conn, actor, arguments).await,
        "find_people" => admin::find_people(conn, actor, arguments).await,
        "org_overview" => admin::org_overview(conn, actor, arguments).await,
        _ => Err("unknown tool".to_owned()),
    };
    match result {
        Ok(value) => value.to_string(),
        Err(message) => error(&message),
    }
}

fn error(message: &str) -> String {
    json!({ "error": message }).to_string()
}

/// A service refused or failed. The model is told no more than that; the cause is logged.
fn unavailable(tool: &str, cause: impl std::fmt::Display) -> String {
    tracing::warn!(tool, error = %cause, "ai_agent: a tool could not read");
    "could not read that: it may not exist, or the user may not have access".to_owned()
}

fn args<T: DeserializeOwned>(raw: &str) -> Result<T, String> {
    let raw = if raw.trim().is_empty() { "{}" } else { raw };
    serde_json::from_str(raw).map_err(|_| "the arguments do not fit this tool".to_owned())
}

/// The first [`MAX_ROWS`] rows as JSON, then a note saying how many were left out.
fn capped<T>(items: &[T], row: impl Fn(&T) -> Value) -> Value {
    let mut rows: Vec<Value> = items.iter().take(MAX_ROWS).map(row).collect();
    if items.len() > MAX_ROWS {
        rows.push(json!({"note": format!("{} more rows not shown", items.len() - MAX_ROWS)}));
    }
    Value::Array(rows)
}

fn course_row(card: &teaching::CardCourse) -> Value {
    json!({"id": card.course.id, "title": card.course.title, "code": card.code})
}

async fn my_work(
    conn: &mut PgConnection,
    actor: &Actor,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    let work = gradebook::my_work(conn, actor, now)
        .await
        .map_err(|e| unavailable("my_work", e))?;
    Ok(json!({
        "courses": capped(&work.courses, course_row),
        "assignments": capped(&work.items, |w| json!({
            "id": w.assignment_id,
            "course_id": w.course_id,
            "title": w.title,
            "type": w.assignment_type.id(),
            "due_at": w.due_at.map(|d| d.to_rfc3339()),
            "class_on": w.class_on.map(|d| d.to_string()),
            "max_points": w.max_points.to_string(),
            "status": w.status.as_str(),
            "score": w.score.map(|s| s.to_string()),
        })),
    }))
}

#[derive(Deserialize)]
struct CourseArg {
    course_id: Uuid,
}

async fn my_grades(
    conn: &mut PgConnection,
    actor: &Actor,
    arguments: &str,
) -> Result<Value, String> {
    let CourseArg { course_id } = args(arguments)?;
    let grades = gradebook::my_grades(conn, actor, course_id)
        .await
        .map_err(|e| unavailable("my_grades", e))?;
    Ok(json!({
        "grades": capped(&grades, |g| json!({
            "assignment_id": g.assignment_id,
            "title": g.title,
            "max_points": g.max_points.to_string(),
            "bonus": g.is_bonus,
            "score": g.score.map(|s| s.to_string()),
            "feedback": g.feedback,
            "published_at": g.published_at.to_rfc3339(),
        })),
    }))
}

/// Students listed, and cells listed for each: what fits in a model's context.
async fn course_gradebook(
    conn: &mut PgConnection,
    actor: &Actor,
    arguments: &str,
) -> Result<Value, String> {
    let CourseArg { course_id } = args(arguments)?;
    let page = gradebook::gradebook(
        conn,
        actor,
        false,
        course_id,
        PageParams {
            after: None,
            limit: MAX_ROWS as u32,
        },
    )
    .await
    .map_err(|e| unavailable("course_gradebook", e))?;
    Ok(json!({
        "assignments": capped(&page.columns, |c| json!({
            "id": c.assignment_id,
            "title": c.title,
            "max_points": c.max_points.to_string(),
            "bonus": c.is_bonus,
            "published": c.status == ColumnStatus::Published,
        })),
        "students": capped(&page.rows, |r| json!({
            "id": r.user_id,
            "name": format!("{} {}", r.name, r.surname),
            "grades": capped(&r.cells, |(assignment, cell)| json!({
                "assignment_id": assignment,
                "draft": cell.draft_score.map(|d| d.to_string()),
                "published": cell.published_score.map(|d| d.to_string()),
                "unpublished_changes": cell.has_unpublished_changes,
            })),
        })),
        "more_students_not_shown": page.next.is_some(),
    }))
}

/// The tree of a course as staff see it: ids to build on, drafts included.
async fn course_content(
    conn: &mut PgConnection,
    actor: &Actor,
    arguments: &str,
) -> Result<Value, String> {
    let CourseArg { course_id } = args(arguments)?;
    let tree = content::get_tree(conn, actor, course_id)
        .await
        .map_err(|e| unavailable("course_content", e))?;
    let item = |i: &content::ItemSummary| json!({"id": i.id, "title": i.title, "kind": i.kind.as_str(), "published": i.status.as_str() == "published"});
    let module = |m: &content::Module| json!({"id": m.id, "title": m.title, "published": m.status.as_str() == "published"});
    Ok(json!({"modules": capped(&tree, |m| {
        let mut row = module(&m.module);
        row["items"] = capped(&m.items, item);
        row["submodules"] = capped(&m.submodules, |s| {
            let mut sub = module(&s.module);
            sub["items"] = capped(&s.items, item);
            sub
        });
        row
    })}))
}

#[derive(Deserialize)]
struct AssignmentArg {
    assignment_id: Uuid,
}

/// One assignment as its student sees it. Only a text assignment's instructions are given: the
/// questions of a quiz stay on its page, so the coach cannot answer them for the student.
async fn assignment_info(
    conn: &mut PgConnection,
    actor: &Actor,
    arguments: &str,
) -> Result<Value, String> {
    let AssignmentArg { assignment_id } = args(arguments)?;
    let item = items::get(conn, actor, assignment_id)
        .await
        .map_err(|e| unavailable("assignment_info", e))?;
    let Some(assignment) = &item.assignment else {
        return Err("that item is not an assignment".to_owned());
    };
    let is_text = assignment.assignment_type.id() == content::text::ID;
    Ok(json!({
        "id": item.id,
        "course_id": item.course_id,
        "title": item.title,
        "type": assignment.assignment_type.id(),
        "max_points": assignment.max_points.to_string(),
        "due_at": assignment.settings.due_at.map(|d| d.to_rfc3339()),
        "instructions": is_text.then(|| text_of(&item.body).chars().take(6000).collect::<String>()),
    }))
}

async fn teaching_overview(
    conn: &mut PgConnection,
    actor: &Actor,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    let o = gradebook::teaching_overview(conn, actor, now)
        .await
        .map_err(|e| unavailable("teaching_overview", e))?;
    Ok(json!({
        "courses": capped(&o.courses, course_row),
        "to_grade": capped(&o.to_grade, |r| json!({
            "student": format!("{} {}", r.student_name, r.student_surname),
            "student_id": r.student_id,
            "assignment_id": r.assignment_id,
            "assignment": r.assignment_title,
            "course_id": r.course_id,
            "submitted_at": r.submitted_at.to_rfc3339(),
            "attempt": r.attempt,
        })),
        "grades_not_published": capped(&o.unpublished, |r| json!({
            "assignment_id": r.assignment_id,
            "assignment": r.assignment_title,
            "course_id": r.course_id,
            "draft_grades": r.draft_count,
        })),
        "deadlines": capped(&o.deadlines, |r| json!({
            "assignment_id": r.assignment_id,
            "assignment": r.title,
            "course_id": r.course_id,
            "due_at": r.due_at.to_rfc3339(),
            "submitted": r.submitted,
            "of": r.total,
        })),
        "content_not_published": capped(&o.unpublished_content, |r| json!({
            "item_id": r.item_id,
            "title": r.title,
            "course_id": r.course_id,
        })),
    }))
}

#[cfg(test)]
mod tests;
