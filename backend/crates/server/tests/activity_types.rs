//! ADR-0097: the work lists ask the capabilities of the installed types, not their names. The
//! registry is process-wide, so this binary installs the built-ins plus a type of its own before
//! any server starts; the other test binaries run on the built-ins alone.

mod common;

use ::common::Role;
use activity::{
    ActivityType, AutoGrade, Capabilities, Claims, CompletionPolicy, ConfigError, DocumentErrors,
    EffectiveProfiles, Stage, Types, ValidContent,
};
use common::{Account, Env, db_test, essay, id_of};
use serde_json::{Map, Value, json};
use sqlx::types::Decimal;
use uuid::Uuid;

/// A type that takes no submissions and settings, like `no_submission`, under another name: a
/// lesson held in a room. Nothing in the core knows it.
struct InPerson;

const IN_PERSON: &str = "in_person";

impl ActivityType for InPerson {
    fn id(&self) -> &'static str {
        IN_PERSON
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            submissions: false,
            auto_graded: false,
        }
    }

    fn completion(&self) -> CompletionPolicy {
        CompletionPolicy::Graded
    }

    fn validate_config(
        &self,
        config: &Map<String, Value>,
        _profiles: &EffectiveProfiles,
    ) -> Result<(), ConfigError> {
        match config.keys().next() {
            None => Ok(()),
            Some(field) => Err(ConfigError::new(format!("unknown field `{field}`"))),
        }
    }

    fn points_total(&self, _config: &Map<String, Value>) -> Option<Decimal> {
        None
    }

    fn strip_computed(&self, _config: &mut Map<String, Value>) {}

    fn validate_submission(
        &self,
        _config: &Map<String, Value>,
        _content: &Value,
        _stage: Stage,
        _profiles: &EffectiveProfiles,
    ) -> Result<ValidContent, DocumentErrors> {
        Err(DocumentErrors::one("", "this type takes no submissions"))
    }

    fn student_config(&self, config: &Map<String, Value>) -> Map<String, Value> {
        config.clone()
    }

    fn structure(&self, _config: &Map<String, Value>) -> Option<Value> {
        None
    }

    fn auto_grade(
        &self,
        _config: &Map<String, Value>,
        _content: &Map<String, Value>,
    ) -> Option<AutoGrade> {
        None
    }

    fn claims(&self, _config: &Map<String, Value>, _content: &Map<String, Value>) -> Claims {
        Claims::default()
    }
}

static IN_PERSON_TYPE: InPerson = InPerson;

/// Installs the registry of this binary: the built-ins and `in_person`. `AppState::new` ignores
/// an installed registry, so this one wins as long as it runs before the first server starts.
fn install_types() {
    activity::ensure_installed(|| {
        content::builtin_types()
            .and_then(|types: Types| types.with(&IN_PERSON_TYPE))
            .expect("the built-in types and in_person")
    });
}

struct World {
    env: Env,
    owner: Account,
    student: Account,
    module: Uuid,
}

async fn world(pool: sqlx::PgPool) -> World {
    install_types();
    let env = Env::new(pool).await;
    let owner = env.user("owner@lms.test", Role::Teacher).await;
    let student = env.user("student@lms.test", Role::Student).await;
    let (_course, module) = env.course(&owner, &[&student]).await;
    World {
        env,
        owner,
        student,
        module,
    }
}

impl World {
    async fn ok(
        &self,
        method: &str,
        who: &Account,
        path: &str,
        body: Option<Value>,
        status: u16,
    ) -> Value {
        self.env.ok(method, &who.token, path, body, status).await
    }

    /// A published assignment; `assignment` is its body without the title.
    async fn assignment(&self, title: &str, assignment: Value) -> Uuid {
        let id = id_of(
            &self
                .ok(
                    "POST",
                    &self.owner,
                    &format!("/modules/{}/items", self.module),
                    Some(json!({"kind": "assignment", "title": title, "assignment": assignment})),
                    201,
                )
                .await,
        );
        self.ok(
            "POST",
            &self.owner,
            &format!("/items/{id}/publish"),
            None,
            200,
        )
        .await;
        id
    }

    async fn text(&self, title: &str) -> Uuid {
        self.assignment(
            title,
            json!({"type": "text", "max_points": "10", "due_at": "2099-01-01T10:00:00Z",
                   "on_deadline": "keep_draft", "accept_late": false}),
        )
        .await
    }

    async fn grade(&self, assignment: Uuid, score: &str) {
        self.ok(
            "PUT",
            &self.owner,
            &format!("/assignments/{assignment}/grades/{}", self.student.id),
            Some(json!({"score": score, "feedback": "note"})),
            200,
        )
        .await;
    }

    async fn first_card(&self, who: &Account) -> Value {
        self.ok("GET", who, "/me/course-cards", None, 200).await["items"][0].clone()
    }
}

#[tokio::test]
async fn submissionless_types_come_from_the_registry() {
    db_test(async |pool| {
        let w = world(pool).await;
        let lesson = w
            .assignment(
                "Held in a room",
                json!({"type": IN_PERSON, "max_points": "5"}),
            )
            .await;
        w.text("Essay").await;

        let work = w.ok("GET", &w.student, "/me/work", None, 200).await;
        let status_of = |id: Uuid| {
            work["items"]
                .as_array()
                .and_then(|items| items.iter().find(|i| i["assignment_id"] == json!(id)))
                .map(|i| i["status"].clone())
        };
        assert_eq!(status_of(lesson), Some(json!("in_class")), "{work}");

        // Progress counts only what is handed in: the essay, not the lesson.
        let card = w.first_card(&w.student).await;
        assert_eq!(
            card["progress"],
            json!({"submitted": 0, "total": 1, "graded": 0}),
            "{card}"
        );
        assert_eq!(card["next"]["title"], "Essay", "{card}");
    })
    .await;
}

#[tokio::test]
async fn auto_graded_types_come_from_the_registry() {
    db_test(async |pool| {
        let w = world(pool).await;
        // A quiz of one number question, taken at once: the attempt is scored into a draft grade.
        let quiz = w
            .assignment(
                "Quiz",
                json!({"type": "quiz", "max_points": "1", "config": {
                    "v": 1, "shuffle_questions": false, "show_correct": true,
                    "questions": [{"id": "q1", "type": "number", "points": "1",
                                   "prompt": essay("pi"), "answer": "3.14", "tolerance": "0.01"}]}}),
            )
            .await;
        let essay_task = w.text("Essay").await;
        w.ok(
            "PUT",
            &w.student,
            &format!("/assignments/{quiz}/submission/draft"),
            Some(json!({"content": {"answers": {"q1": "3.14"}}})),
            200,
        )
        .await;
        w.ok(
            "POST",
            &w.student,
            &format!("/assignments/{quiz}/submission/submit"),
            None,
            201,
        )
        .await;
        w.grade(essay_task, "4").await;

        let view = w.ok("GET", &w.owner, "/me/teaching", None, 200).await;
        let auto_scored = |title: &str| {
            view["unpublished"]
                .as_array()
                .and_then(|rows| rows.iter().find(|r| r["assignment_title"] == title))
                .map(|r| r["auto_scored"].clone())
        };
        assert_eq!(auto_scored("Quiz"), Some(json!(true)), "{view}");
        assert_eq!(auto_scored("Essay"), Some(json!(false)), "{view}");

        // The card flags the draft grades as auto-scored while a quiz is among them.
        let todo = w.first_card(&w.owner).await["todo"].clone();
        let drafts = todo
            .as_array()
            .and_then(|all| all.iter().find(|t| t["kind"] == "draft_grades"))
            .cloned();
        assert_eq!(
            drafts,
            Some(json!({"kind": "draft_grades", "count": 2, "auto_scored": true})),
            "{todo}"
        );
    })
    .await;
}
