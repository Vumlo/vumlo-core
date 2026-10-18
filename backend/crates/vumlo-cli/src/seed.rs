//! `vumlo-cli seed --demo`: a small institution to look at — one course, a teacher, three students,
//! a lesson, a quiz and published grades.
//!
//! The CLI writes the rows itself: it cannot call the core, which may not be running, and it must
//! not depend on it. So these inserts follow what the API writes, and `tests/seed.rs` checks the
//! result through the real server: the students see the course and the quiz without its answer
//! key, the grades are published, and the quiz can be taken and scored.

use anyhow::{Context, bail};
use db::identity::{Email, PersonName, Role, TemporaryPassword};
use serde_json::{Value, json};
use sqlx::types::Decimal;
use uuid::Uuid;
use vumlo_config::Options;

use crate::runtime;

/// The course code that marks the demo data: a second run finds it and does nothing.
pub const DEMO_COURSE_CODE: &str = "DEMO-101";

struct Person {
    email: &'static str,
    name: &'static str,
    surname: &'static str,
    role: Role,
}

const TEACHER: Person = Person {
    email: "maria.kovalenko@demo.vumlo.example",
    name: "Maria",
    surname: "Kovalenko",
    role: Role::Teacher,
};

const STUDENTS: [(Person, &str, &str); 3] = [
    (
        Person {
            email: "olena.shevchenko@demo.vumlo.example",
            name: "Olena",
            surname: "Shevchenko",
            role: Role::Student,
        },
        "10",
        "Full marks. Well done.",
    ),
    (
        Person {
            email: "taras.bondar@demo.vumlo.example",
            name: "Taras",
            surname: "Bondar",
            role: Role::Student,
        },
        "7",
        "Good. Look again at question 2.",
    ),
    (
        Person {
            email: "iryna.melnyk@demo.vumlo.example",
            name: "Iryna",
            surname: "Melnyk",
            role: Role::Student,
        },
        "4",
        "Come and see me before the next lesson.",
    ),
];

/// A block document of the full profile with one paragraph (ADR-0025).
fn doc(text: &str) -> Value {
    json!({"v": 1, "attachments": [], "blocks": [
        {"id": "p1", "type": "paragraph", "content": [{"type": "text", "text": text}]}]})
}

/// A quiz worth ten points: a single choice (6) and a number (4). Ten fits both grading scales.
fn quiz_config() -> Value {
    json!({
        "v": 1, "shuffle_questions": false, "show_correct": true,
        "questions": [
            {"id": "q1", "type": "single_choice", "points": "6", "prompt": doc("Which of these is a prime number?"),
             "options": [
                 {"id": "a", "content": doc("9")},
                 {"id": "b", "content": doc("11")},
                 {"id": "c", "content": doc("15")},
             ],
             "correct": ["b"], "explanation": doc("11 has no divisors other than 1 and itself.")},
            {"id": "q2", "type": "number", "points": "4", "prompt": doc("What is 6 times 7?"),
             "answer": "42", "tolerance": "0"},
        ]
    })
}

pub struct SeedDemo {
    pub yes: bool,
    pub dry_run: bool,
}

pub async fn demo(options: &Options, args: SeedDemo) -> anyhow::Result<()> {
    let loaded = runtime::load(options)?;
    let settings = &loaded.settings;
    let pool = runtime::connect(settings).await?;
    db::ensure_migrated_with(&pool, &runtime::migrators())
        .await
        .context("the database is not ready")?;

    let existing = sqlx::query_scalar!(
        r#"SELECT id AS "id!" FROM teaching.courses WHERE code = $1"#,
        DEMO_COURSE_CODE
    )
    .fetch_optional(&pool)
    .await?;
    if existing.is_some() {
        println!("the demo data is already there ({DEMO_COURSE_CODE}); nothing was changed");
        return Ok(());
    }

    let courses = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM teaching.courses"#)
        .fetch_one(&pool)
        .await?;
    let people =
        sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM identity.users WHERE role <> 'admin'"#)
            .fetch_one(&pool)
            .await?;
    if (courses > 0 || people > 0) && !args.yes {
        bail!(
            "this installation is not empty ({courses} course(s), {people} user(s) who are not \
             administrators); demo data belongs in a fresh one. Pass --yes to add it anyway"
        );
    }
    if args.dry_run {
        println!(
            "would create: 1 teacher, 3 students, the course {DEMO_COURSE_CODE} with a lesson,"
        );
        println!("a quiz worth 10 points and 3 published grades");
        return Ok(());
    }

    // Hash before the transaction opens (ADR-0016): four passwords, shown once at the end.
    let people: Vec<(&Person, TemporaryPassword)> = std::iter::once(&TEACHER)
        .chain(STUDENTS.iter().map(|(person, _, _)| person))
        .map(|person| (person, TemporaryPassword::generate()))
        .collect();
    let mut hashes = Vec::new();
    for (_, password) in &people {
        let plain = password.expose().to_owned();
        let hash = tokio::task::spawn_blocking(move || db::identity::hash(&plain))
            .await
            .context("the hashing task failed")?
            .map_err(|e| anyhow::anyhow!("cannot hash a password: {e}"))?;
        hashes.push(hash);
    }

    let mut tx = pool.begin().await.context("cannot start a transaction")?;
    db::record_mode(&mut tx, settings.institution.mode).await?;

    let mut ids = Vec::new();
    for ((person, _), hash) in people.iter().zip(&hashes) {
        let id = Uuid::now_v7();
        let email = Email::parse(person.email).context("a demo email")?;
        let name = PersonName::name(person.name).context("a demo name")?;
        let surname = PersonName::surname(person.surname).context("a demo surname")?;
        sqlx::query!(
            "INSERT INTO identity.users (id, email, name, surname, role, must_change_password, email_verified_at)
             VALUES ($1, $2, $3, $4, $5, true, now())",
            id,
            email.as_str(),
            name.as_str(),
            surname.as_str(),
            person.role.as_str()
        )
        .execute(&mut *tx)
        .await
        .context("cannot create a demo user (does one of them exist already?)")?;
        sqlx::query!(
            "INSERT INTO identity.identities (id, user_id, provider, subject, password_hash)
             VALUES ($1, $2, 'password', $3, $4)",
            Uuid::now_v7(),
            id,
            id.to_string(),
            hash.as_str()
        )
        .execute(&mut *tx)
        .await?;
        ids.push(id);
    }
    let teacher = ids[0];

    let course = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO teaching.courses (id, title, status, owner_id, created_by, code, description)
         VALUES ($1, 'Introduction to Demonstration', 'published', $2, $2, $3,
                 'A small course with demo data: a lesson, a quiz and some grades.')",
        course,
        teacher,
        DEMO_COURSE_CODE
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO teaching.memberships (course_id, user_id, role) VALUES ($1, $2, 'teacher')",
        course,
        teacher
    )
    .execute(&mut *tx)
    .await?;
    for student in &ids[1..] {
        sqlx::query!(
            "INSERT INTO teaching.memberships (course_id, user_id, role) VALUES ($1, $2, 'student')",
            course,
            student
        )
        .execute(&mut *tx)
        .await?;
    }

    let module = Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO content.modules (id, course_id, position, title, status)
         VALUES ($1, $2, 1, 'Week 1', 'published')",
        module,
        course
    )
    .execute(&mut *tx)
    .await?;

    let text = Uuid::now_v7();
    let quiz = Uuid::now_v7();
    for (id, position, kind, title, body) in [
        (
            text,
            1,
            "text",
            "Welcome",
            doc("Welcome to the demo course. Start with the quiz."),
        ),
        (
            quiz,
            2,
            "assignment",
            "Warm-up quiz",
            doc("Ten points, two questions."),
        ),
    ] {
        sqlx::query!(
            "INSERT INTO content.items
                 (id, module_id, course_id, position, kind, title, body, status,
                  published_title, published_body, last_published_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'published', $6, $7, now())",
            id,
            module,
            course,
            position,
            kind,
            title,
            body
        )
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query!(
        "INSERT INTO content.assignments (item_id, type, config, max_points)
         VALUES ($1, 'quiz', $2, 10)",
        quiz,
        quiz_config()
    )
    .execute(&mut *tx)
    .await?;

    for (student, (_, score, feedback)) in ids[1..].iter().zip(STUDENTS.iter()) {
        let score: Decimal = score.parse().context("a demo score")?;
        sqlx::query!(
            "INSERT INTO gradebook.grades
                 (assignment_id, student_id, draft_score, draft_feedback, draft_updated_by,
                  published_score, published_feedback, published_by, published_at)
             VALUES ($1, $2, $3, $4, $5, $3, $4, $5, now())",
            quiz,
            student,
            score,
            *feedback,
            teacher
        )
        .execute(&mut *tx)
        .await?;
        for action in ["save_draft", "publish"] {
            sqlx::query!(
                "INSERT INTO gradebook.grade_history
                     (id, assignment_id, student_id, action, actor_id, score, feedback)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                Uuid::now_v7(),
                quiz,
                student,
                action,
                teacher,
                score,
                *feedback
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    db::insert_cli_audit(
        &mut tx,
        "demo.seed",
        "course",
        course,
        json!({"users": ids.len(), "items": 2, "grades": STUDENTS.len()}),
    )
    .await?;
    tx.commit().await.context("cannot commit")?;

    println!(
        "created the course {DEMO_COURSE_CODE} with a lesson, a quiz and {} published grades",
        STUDENTS.len()
    );
    println!();
    println!("Accounts (shown once; each must change the password at the first sign-in):");
    for (person, password) in &people {
        println!(
            "  {:<8} {:<40} {}",
            person.role.as_str(),
            person.email,
            password.expose()
        );
    }
    Ok(())
}
