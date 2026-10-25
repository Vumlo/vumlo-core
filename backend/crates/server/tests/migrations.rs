mod common;

use common::{bare_db_test, db_test};
use db::MigrationCheckError;
use sqlx::PgPool;

#[tokio::test]
async fn migrations_create_exactly_the_module_schemas() {
    db_test(async |pool| {
        let schemas = sqlx::query_scalar!(
            r#"SELECT schema_name AS "name!" FROM information_schema.schemata
               WHERE schema_name NOT LIKE 'pg\_%'
                 AND schema_name NOT IN ('information_schema', 'public')
               ORDER BY schema_name"#
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        assert_eq!(
            schemas,
            [
                "assessment",
                "auth",
                "content",
                "events",
                "files",
                "gradebook",
                "idempotency",
                "org",
                "teaching"
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn serve_check_refuses_an_unmigrated_database() {
    bare_db_test(async |pool| {
        let error = db::ensure_migrated(&pool).await.unwrap_err();

        let embedded = db::MIGRATOR.iter().count();
        assert!(
            matches!(error, MigrationCheckError::Pending(n) if n == embedded),
            "{error}"
        );
        assert!(
            error.to_string().contains("vumlo-server migrate"),
            "{error}"
        );

        db::MIGRATOR.run(&pool).await.unwrap();

        db::ensure_migrated(&pool).await.unwrap();
    })
    .await;
}

async fn insert_user(pool: &PgPool, email: &str, role: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO auth.users (id, email, name, surname, role) VALUES ($1, $2, 'Test', 'User', $3)",
        uuid::Uuid::now_v7(),
        email,
        role
    )
    .execute(pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn identity_schema_rejects_invalid_users() {
    db_test(async |pool| {
        insert_user(&pool, "a@x.edu", "student").await.unwrap();

        assert!(insert_user(&pool, "a@x.edu", "student").await.is_err());
        assert!(
            insert_user(&pool, "A@x.edu", "student").await.is_err(),
            "emails must be stored lowercase"
        );
        assert!(insert_user(&pool, "b@x.edu", "owner").await.is_err());
    })
    .await;
}

#[tokio::test]
async fn deleting_a_user_cascades_to_identities_and_sessions() {
    db_test(async |pool| {
        let user_id = uuid::Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO auth.users (id, email, name, surname, role) VALUES ($1, 'c@x.edu', 'C', 'D', 'teacher')",
            user_id
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO auth.identities (id, user_id, provider, subject, password_hash)
             VALUES ($1, $2, 'password', $3, 'hash')",
            uuid::Uuid::now_v7(),
            user_id,
            user_id.to_string()
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO auth.sessions (id, user_id, token_hash, kind, expires_at)
             VALUES ($1, $2, $3, 'web', now() + interval '1 day')",
            uuid::Uuid::now_v7(),
            user_id,
            vec![0u8; 32]
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query!("DELETE FROM auth.users WHERE id = $1", user_id)
            .execute(&pool)
            .await
            .unwrap();

        let left = sqlx::query_scalar!(
            r#"SELECT (SELECT count(*) FROM auth.identities)
                    + (SELECT count(*) FROM auth.sessions) AS "n!""#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(left, 0);
    })
    .await;
}

#[tokio::test]
async fn password_identity_requires_a_hash_and_google_must_not_have_one() {
    db_test(async |pool| {
        insert_user(&pool, "d@x.edu", "student").await.unwrap();
        let user_id = sqlx::query_scalar!(r#"SELECT id AS "id!" FROM auth.users"#)
            .fetch_one(&pool)
            .await
            .unwrap();

        let no_hash = sqlx::query!(
            "INSERT INTO auth.identities (id, user_id, provider, subject)
             VALUES ($1, $2, 'password', 's1')",
            uuid::Uuid::now_v7(),
            user_id
        )
        .execute(&pool)
        .await;
        let google_with_hash = sqlx::query!(
            "INSERT INTO auth.identities (id, user_id, provider, subject, password_hash)
             VALUES ($1, $2, 'google', 's2', 'h')",
            uuid::Uuid::now_v7(),
            user_id
        )
        .execute(&pool)
        .await;

        assert!(no_hash.is_err());
        assert!(google_with_hash.is_err());
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// content (ADR-0020)
// ---------------------------------------------------------------------------------------------

async fn insert_course(pool: &PgPool, owner: uuid::Uuid) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO teaching.courses (id, title, owner_id, created_by) VALUES ($1, 'Course', $2, $2)",
        id,
        owner
    )
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn insert_module_row(
    pool: &PgPool,
    course_id: uuid::Uuid,
    parent_id: Option<uuid::Uuid>,
    position: i32,
) -> Result<uuid::Uuid, sqlx::Error> {
    let id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO content.modules (id, course_id, parent_id, position, title)
         VALUES ($1, $2, $3, $4, 'Module')",
        id,
        course_id,
        parent_id,
        position
    )
    .execute(pool)
    .await?;
    Ok(id)
}

async fn insert_item_row(
    pool: &PgPool,
    module_id: uuid::Uuid,
    course_id: uuid::Uuid,
    position: i32,
    body: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO content.items (id, module_id, course_id, position, kind, title, body)
         VALUES ($1, $2, $3, $4, 'text', 'Text', $5)",
        uuid::Uuid::now_v7(),
        module_id,
        course_id,
        position,
        body
    )
    .execute(pool)
    .await
    .map(|_| ())
}

fn sqlstate(result: Result<impl std::fmt::Debug, sqlx::Error>) -> String {
    let error = result.expect_err("the database should refuse this");
    error
        .as_database_error()
        .and_then(|e| e.code())
        .unwrap_or_default()
        .into_owned()
}

async fn content_row_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM content.modules) + (SELECT count(*) FROM content.items) AS "n!""#
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn owner(pool: &PgPool, email: &str) -> uuid::Uuid {
    insert_user(pool, email, "teacher").await.unwrap();
    sqlx::query_scalar!("SELECT id FROM auth.users WHERE email = $1", email)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn content_modules_keep_one_level_and_one_course() {
    db_test(async |pool| {
        let owner = owner(&pool, "t@x.edu").await;
        let course = insert_course(&pool, owner).await;
        let other_course = insert_course(&pool, owner).await;
        let top = insert_module_row(&pool, course, None, 1).await.unwrap();
        let other_top = insert_module_row(&pool, other_course, None, 1)
            .await
            .unwrap();

        // A sub-module of a top-level module of the same course is fine.
        let sub = insert_module_row(&pool, course, Some(top), 1)
            .await
            .unwrap();
        // The parent belongs to another course.
        assert_eq!(
            sqlstate(insert_module_row(&pool, course, Some(other_top), 1).await),
            "23503"
        );
        // The parent is itself a sub-module: depth would be two.
        assert_eq!(
            sqlstate(insert_module_row(&pool, course, Some(sub), 1).await),
            "23503"
        );
        // An unknown parent.
        assert_eq!(
            sqlstate(insert_module_row(&pool, course, Some(uuid::Uuid::now_v7()), 1).await),
            "23503"
        );

        insert_item_row(&pool, sub, course, 1, serde_json::json!({}))
            .await
            .unwrap();
        insert_item_row(&pool, top, course, 1, serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(content_row_count(&pool).await, 5);

        // Deleting the top-level module takes its sub-module and every item below with it.
        sqlx::query!("DELETE FROM content.modules WHERE id = $1", top)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            content_row_count(&pool).await,
            1,
            "only the other course's module is left"
        );
    })
    .await;
}

#[tokio::test]
async fn content_positions_are_unique_per_sequence_and_checked_at_commit() {
    db_test(async |pool| {
        let owner = owner(&pool, "t@x.edu").await;
        let course = insert_course(&pool, owner).await;
        let first = insert_module_row(&pool, course, None, 1).await.unwrap();
        let second = insert_module_row(&pool, course, None, 2).await.unwrap();

        // Top-level modules have a NULL parent; the constraint still treats them as one sequence.
        assert_eq!(
            sqlstate(insert_module_row(&pool, course, None, 1).await),
            "23505"
        );
        // Sub-modules are a different sequence from their parent's siblings.
        let sub = insert_module_row(&pool, course, Some(first), 1)
            .await
            .unwrap();
        insert_module_row(&pool, course, Some(second), 1)
            .await
            .unwrap();
        assert_eq!(
            sqlstate(insert_module_row(&pool, course, Some(first), 1).await),
            "23505"
        );

        // Two rows swap places in one statement: only the end state is checked.
        sqlx::query!(
            "UPDATE content.modules SET position = 3 - position WHERE id IN ($1, $2)",
            first,
            second
        )
        .execute(&pool)
        .await
        .unwrap();

        insert_item_row(&pool, sub, course, 1, serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            sqlstate(insert_item_row(&pool, sub, course, 1, serde_json::json!({})).await),
            "23505"
        );
    })
    .await;
}

#[tokio::test]
async fn content_items_must_belong_to_their_modules_course() {
    db_test(async |pool| {
        let owner = owner(&pool, "t@x.edu").await;
        let course = insert_course(&pool, owner).await;
        let other_course = insert_course(&pool, owner).await;
        let module = insert_module_row(&pool, course, None, 1).await.unwrap();

        assert_eq!(
            sqlstate(insert_item_row(&pool, module, other_course, 1, serde_json::json!({})).await),
            "23503"
        );
        insert_item_row(&pool, module, course, 1, serde_json::json!({"a": [1, 2]}))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn content_item_body_must_be_a_json_object() {
    db_test(async |pool| {
        let owner = owner(&pool, "t@x.edu").await;
        let course = insert_course(&pool, owner).await;
        let module = insert_module_row(&pool, course, None, 1).await.unwrap();

        for (position, body) in [
            serde_json::json!([1]),
            serde_json::json!("text"),
            serde_json::json!(null),
        ]
        .into_iter()
        .enumerate()
        {
            let position = i32::try_from(position).unwrap() + 1;
            assert_eq!(
                sqlstate(insert_item_row(&pool, module, course, position, body).await),
                "23514"
            );
        }
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// assignments (ADR-0021)
// ---------------------------------------------------------------------------------------------

async fn insert_item_of_kind(
    pool: &PgPool,
    module_id: uuid::Uuid,
    course_id: uuid::Uuid,
    position: i32,
    kind: &str,
) -> Result<uuid::Uuid, sqlx::Error> {
    let id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO content.items (id, module_id, course_id, position, kind, title)
         VALUES ($1, $2, $3, $4, $5, 'Item')",
        id,
        module_id,
        course_id,
        position,
        kind
    )
    .execute(pool)
    .await?;
    Ok(id)
}

/// `max_points` is passed as text so each test can name the exact literal PostgreSQL parses.
async fn insert_assignment_row(
    pool: &PgPool,
    item_id: uuid::Uuid,
    max_points: &str,
    config: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO content.assignments (item_id, type, config, max_points)
         VALUES ($1, 'text', $2, $3::text::numeric)",
        item_id,
        config,
        max_points
    )
    .execute(pool)
    .await
    .map(|_| ())
}

/// A course with one module, and an assignment item and a `text` item in it.
async fn assignment_fixture(pool: &PgPool) -> (uuid::Uuid, uuid::Uuid) {
    let owner = owner(pool, "t@x.edu").await;
    let course = insert_course(pool, owner).await;
    let module = insert_module_row(pool, course, None, 1).await.unwrap();
    let assignment = insert_item_of_kind(pool, module, course, 1, "assignment")
        .await
        .unwrap();
    let text = insert_item_of_kind(pool, module, course, 2, "text")
        .await
        .unwrap();
    (assignment, text)
}

#[tokio::test]
async fn item_kind_accepts_assignment_and_rejects_anything_else() {
    db_test(async |pool| {
        let owner = owner(&pool, "t@x.edu").await;
        let course = insert_course(&pool, owner).await;
        let module = insert_module_row(&pool, course, None, 1).await.unwrap();

        insert_item_of_kind(&pool, module, course, 1, "assignment")
            .await
            .unwrap();
        insert_item_of_kind(&pool, module, course, 2, "text")
            .await
            .unwrap();
        for (position, kind) in [(3, "quiz"), (4, "lesson")] {
            assert_eq!(
                sqlstate(insert_item_of_kind(&pool, module, course, position, kind).await),
                "23514",
                "{kind}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn an_assignment_row_belongs_to_an_assignment_item_only() {
    db_test(async |pool| {
        let (assignment, text) = assignment_fixture(&pool).await;

        assert_eq!(
            sqlstate(insert_assignment_row(&pool, text, "5", serde_json::json!({})).await),
            "23503",
            "a text item cannot have an assignment row"
        );
        assert_eq!(
            sqlstate(
                insert_assignment_row(&pool, uuid::Uuid::now_v7(), "5", serde_json::json!({}))
                    .await
            ),
            "23503",
            "nor can an unknown item"
        );
        insert_assignment_row(&pool, assignment, "5", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            sqlstate(insert_assignment_row(&pool, assignment, "6", serde_json::json!({})).await),
            "23505",
            "an item has one assignment row"
        );
    })
    .await;
}

#[tokio::test]
async fn an_item_with_an_assignment_row_cannot_change_kind() {
    db_test(async |pool| {
        let (assignment, _) = assignment_fixture(&pool).await;
        insert_assignment_row(&pool, assignment, "5", serde_json::json!({}))
            .await
            .unwrap();

        let change = sqlx::query!(
            "UPDATE content.items SET kind = 'text' WHERE id = $1",
            assignment
        )
        .execute(&pool)
        .await;
        assert_eq!(sqlstate(change), "23503");
    })
    .await;
}

#[tokio::test]
async fn assignment_points_are_above_zero_and_fit_numeric_6_2() {
    db_test(async |pool| {
        let (assignment, _) = assignment_fixture(&pool).await;
        let json = serde_json::json!({});

        for bad in ["0", "-1", "-0.01"] {
            assert_eq!(
                sqlstate(insert_assignment_row(&pool, assignment, bad, json.clone()).await),
                "23514",
                "{bad}"
            );
        }
        assert_eq!(
            sqlstate(insert_assignment_row(&pool, assignment, "10000", json.clone()).await),
            "22003"
        );
        insert_assignment_row(&pool, assignment, "9999.99", json)
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn assignment_config_must_be_a_json_object() {
    db_test(async |pool| {
        let (assignment, _) = assignment_fixture(&pool).await;

        for bad in [
            serde_json::json!([]),
            serde_json::json!("text"),
            serde_json::json!(null),
        ] {
            assert_eq!(
                sqlstate(insert_assignment_row(&pool, assignment, "5", bad).await),
                "23514"
            );
        }
        insert_assignment_row(&pool, assignment, "5", serde_json::json!({"a": 1}))
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn assignments_are_not_bonus_by_default_and_go_with_their_item() {
    db_test(async |pool| {
        let (assignment, _) = assignment_fixture(&pool).await;
        insert_assignment_row(&pool, assignment, "5", serde_json::json!({}))
            .await
            .unwrap();

        let is_bonus = sqlx::query_scalar!(
            "SELECT is_bonus FROM content.assignments WHERE item_id = $1",
            assignment
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!is_bonus);

        sqlx::query!("DELETE FROM content.items WHERE id = $1", assignment)
            .execute(&pool)
            .await
            .unwrap();
        let rows = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM content.assignments"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    })
    .await;
}

#[tokio::test]
async fn deleting_a_module_removes_its_assignment_rows() {
    db_test(async |pool| {
        let owner = owner(&pool, "t@x.edu").await;
        let course = insert_course(&pool, owner).await;
        let module = insert_module_row(&pool, course, None, 1).await.unwrap();
        let item = insert_item_of_kind(&pool, module, course, 1, "assignment")
            .await
            .unwrap();
        insert_assignment_row(&pool, item, "5", serde_json::json!({}))
            .await
            .unwrap();

        sqlx::query!("DELETE FROM content.modules WHERE id = $1", module)
            .execute(&pool)
            .await
            .unwrap();
        let rows = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM content.assignments"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    })
    .await;
}

async fn item_kind(pool: &PgPool, id: uuid::Uuid) -> String {
    sqlx::query_scalar!("SELECT kind FROM content.items WHERE id = $1", id)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A user as the schema before migration 32 stores them: in the `identity` schema. A runtime
/// query, because the compile-time macros are checked against the full schema, where it is `auth`.
async fn insert_user_before_auth(pool: &PgPool, email: &str, role: &str) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO identity.users (id, email, name, surname, role) VALUES ($1, $2, 'Test', 'User', $3)",
    )
    .bind(id)
    .bind(email)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    id
}

/// A teacher as the schema before migration 10 stores them: one `display_name`. A runtime query,
/// because the compile-time macros are checked against the full schema, where that column is gone.
/// A runtime query on purpose: it writes `display_name`, a column the current schema no longer
/// has, in a test that stops the migrations before it is renamed.
async fn insert_user_before_names(
    pool: &PgPool,
    email: &str,
    display_name: &str,
    role: &str,
) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO identity.users (id, email, display_name, role) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(email)
    .bind(display_name)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn migration_7_turns_lesson_items_into_text_items() {
    bare_db_test(async |pool| {
        db::MIGRATOR.run_to(6, &pool).await.unwrap();

        // Data as it exists before the rename: a `lesson` item and an assignment with its row.
        // The macros are checked against the full schema, but only name columns that exist at 6.
        let owner = insert_user_before_names(&pool, "t@x.edu", "Teacher", "teacher").await;
        let course = insert_course(&pool, owner).await;
        let module = insert_module_row(&pool, course, None, 1).await.unwrap();
        let lesson = insert_item_of_kind(&pool, module, course, 1, "lesson")
            .await
            .unwrap();
        let assignment = insert_item_of_kind(&pool, module, course, 2, "assignment")
            .await
            .unwrap();
        insert_assignment_row(&pool, assignment, "5", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            sqlstate(insert_item_of_kind(&pool, module, course, 3, "text").await),
            "23514",
            "version 6 does not know the kind yet"
        );

        db::MIGRATOR.run(&pool).await.unwrap();

        assert_eq!(item_kind(&pool, lesson).await, "text");
        assert_eq!(item_kind(&pool, assignment).await, "assignment");
        let rows = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM content.assignments"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 1, "the assignment row survived");
        assert_eq!(
            sqlstate(insert_item_of_kind(&pool, module, course, 3, "lesson").await),
            "23514",
            "the new CHECK refuses the old kind"
        );
        insert_item_of_kind(&pool, module, course, 3, "text")
            .await
            .unwrap();
        db::ensure_migrated(&pool).await.unwrap();
    })
    .await;
}

const EMPTY_DOCUMENT: &str = r#"{"v": 1, "blocks": [], "attachments": []}"#;

async fn item_body(pool: &PgPool, id: uuid::Uuid) -> serde_json::Value {
    sqlx::query_scalar!(
        r#"SELECT body AS "body!" FROM content.items WHERE id = $1"#,
        id
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn an_item_without_a_body_starts_with_the_empty_document() {
    db_test(async |pool| {
        let (_, text) = assignment_fixture(&pool).await;
        let empty: serde_json::Value = serde_json::from_str(EMPTY_DOCUMENT).unwrap();
        assert_eq!(item_body(&pool, text).await, empty);
    })
    .await;
}

#[tokio::test]
async fn migration_8_turns_empty_object_bodies_into_the_empty_document() {
    bare_db_test(async |pool| {
        db::MIGRATOR.run_to(7, &pool).await.unwrap();

        let owner = insert_user_before_names(&pool, "t@x.edu", "Teacher", "teacher").await;
        let course = insert_course(&pool, owner).await;
        let module = insert_module_row(&pool, course, None, 1).await.unwrap();
        let empty = insert_item_of_kind(&pool, module, course, 1, "text")
            .await
            .unwrap();
        let kept = insert_item_of_kind(&pool, module, course, 2, "text")
            .await
            .unwrap();
        sqlx::query!(
            "UPDATE content.items SET body = $2 WHERE id = $1",
            kept,
            serde_json::json!({"a": 1})
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(item_body(&pool, empty).await, serde_json::json!({}));

        db::MIGRATOR.run(&pool).await.unwrap();

        let document: serde_json::Value = serde_json::from_str(EMPTY_DOCUMENT).unwrap();
        assert_eq!(item_body(&pool, empty).await, document);
        assert_eq!(
            item_body(&pool, kept).await,
            serde_json::json!({"a": 1}),
            "only the empty object is converted"
        );
        let later = insert_item_of_kind(&pool, module, course, 3, "text")
            .await
            .unwrap();
        assert_eq!(item_body(&pool, later).await, document);
        db::ensure_migrated(&pool).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn migration_10_splits_display_names_into_name_and_surname() {
    bare_db_test(async |pool| {
        db::MIGRATOR.run_to(9, &pool).await.unwrap();

        let cases = [
            ("a@x.edu", "Anna Kowalska", "Anna", "Kowalska"),
            ("b@x.edu", "Anna van der Berg", "Anna", "van der Berg"),
            ("c@x.edu", "Jan \t  Nowak", "Jan", "Nowak"),
            ("d@x.edu", "Madonna", "Madonna", "Madonna"),
        ];
        for (email, display_name, _, _) in cases {
            insert_user_before_names(&pool, email, display_name, "student").await;
        }

        db::MIGRATOR.run(&pool).await.unwrap();

        for (email, _, name, surname) in cases {
            let row = sqlx::query!(
                "SELECT name, surname FROM auth.users WHERE email = $1",
                email
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(
                (row.name.as_str(), row.surname.as_str()),
                (name, surname),
                "{email}"
            );
        }
        let column = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM information_schema.columns
               WHERE table_schema = 'auth' AND table_name = 'users' AND column_name = 'display_name'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(column, 0, "display_name is dropped");
        db::ensure_migrated(&pool).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_user_needs_a_trimmed_name_and_surname_of_at_most_a_hundred_characters() {
    db_test(async |pool| {
        let insert = |name: String, surname: String| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "INSERT INTO auth.users (id, email, name, surname, role)
                     VALUES ($1, $2, $3, $4, 'student')",
                    uuid::Uuid::now_v7(),
                    format!("{}@x.edu", uuid::Uuid::now_v7()),
                    name,
                    surname
                )
                .execute(&pool)
                .await
            }
        };
        insert("Anna".into(), "K".into()).await.unwrap();
        for (name, surname) in [
            ("", "K"),
            ("Anna", ""),
            (" Anna", "K"),
            ("Anna", "K "),
            (&"x".repeat(101), "K"),
            ("Anna", &"x".repeat(101)),
        ] {
            assert_eq!(
                sqlstate(insert(name.to_owned(), surname.to_owned()).await),
                "23514",
                "{name:?} {surname:?}"
            );
        }
    })
    .await;
}

// --- migration 11: the gradebook ----------------------------------------------------------------

async fn insert_grade(
    pool: &PgPool,
    assignment: uuid::Uuid,
    student: uuid::Uuid,
    draft_score: Option<&str>,
    draft_feedback: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO gradebook.grades
             (assignment_id, student_id, draft_score, draft_feedback, draft_updated_by)
         VALUES ($1, $2, $3::text::numeric, $4, $2)",
        assignment,
        student,
        draft_score,
        draft_feedback
    )
    .execute(pool)
    .await
    .map(|_| ())
}

#[tokio::test]
async fn a_grade_row_checks_scores_and_feedback_but_not_the_assignment() {
    db_test(async |pool| {
        let student = owner(&pool, "s@x.edu").await;
        let assignment = uuid::Uuid::now_v7();

        // No foreign key to the assignment (ADR-0021): the gradebook's guards keep them consistent.
        insert_grade(&pool, assignment, student, Some("0"), Some("fine"))
            .await
            .unwrap();
        assert_eq!(
            sqlstate(insert_grade(&pool, assignment, student, Some("1"), None).await),
            "23505",
            "one row per (assignment, student)"
        );
        let other = uuid::Uuid::now_v7();
        assert_eq!(
            sqlstate(insert_grade(&pool, other, student, Some("-0.01"), None).await),
            "23514"
        );
        let long = "x".repeat(4001);
        let ok_long = "x".repeat(4000);
        for bad in ["", " padded", "padded ", long.as_str()] {
            assert_eq!(
                sqlstate(insert_grade(&pool, other, student, None, Some(bad)).await),
                "23514",
                "{bad:?}"
            );
        }
        insert_grade(&pool, other, student, Some("9999.99"), Some(&ok_long))
            .await
            .unwrap();
        assert_eq!(
            sqlstate(
                insert_grade(
                    &pool,
                    uuid::Uuid::now_v7(),
                    uuid::Uuid::now_v7(),
                    None,
                    None
                )
                .await
            ),
            "23503",
            "the student is a user"
        );
    })
    .await;
}

#[tokio::test]
async fn published_by_and_published_at_go_together() {
    db_test(async |pool| {
        let student = owner(&pool, "s@x.edu").await;
        insert_grade(&pool, uuid::Uuid::now_v7(), student, Some("1"), None)
            .await
            .unwrap();
        let only_by = sqlx::query!("UPDATE gradebook.grades SET published_by = $1", student)
            .execute(&pool)
            .await;
        assert_eq!(sqlstate(only_by), "23514");
        sqlx::query!(
            "UPDATE gradebook.grades SET published_by = $1, published_at = now()",
            student
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn the_grade_history_is_append_only() {
    db_test(async |pool| {
        let actor = owner(&pool, "a@x.edu").await;
        let assignment = uuid::Uuid::now_v7();
        let insert = |action: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "INSERT INTO gradebook.grade_history
                         (id, assignment_id, student_id, action, actor_id, score)
                     VALUES ($1, $2, $3, $4, $3, 5)",
                    uuid::Uuid::now_v7(),
                    assignment,
                    actor,
                    action
                )
                .execute(&pool)
                .await
            }
        };
        insert("save_draft").await.unwrap();
        insert("publish").await.unwrap();
        assert_eq!(sqlstate(insert("approve").await), "23514");

        let update = sqlx::query!("UPDATE gradebook.grade_history SET score = 6")
            .execute(&pool)
            .await;
        assert_eq!(sqlstate(update), "P0001", "a row is never changed");
        let rows = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM gradebook.grade_history WHERE score = 5"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, 2);

        // Deleting stays possible: the purge that goes with the deletion of an assignment uses it.
        let deleted = sqlx::query!(
            "DELETE FROM gradebook.grade_history WHERE assignment_id = $1",
            assignment
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(deleted.rows_affected(), 2);
    })
    .await;
}

// --- migration 12: the effective_members view ---------------------------------------------------

async fn effective_members(pool: &PgPool) -> Vec<(uuid::Uuid, uuid::Uuid, String)> {
    let mut rows: Vec<_> =
        sqlx::query!("SELECT course_id, user_id, role FROM teaching.effective_members")
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| (r.course_id.unwrap(), r.user_id.unwrap(), r.role.unwrap()))
            .collect();
    rows.sort();
    rows
}

async fn memberships(pool: &PgPool) -> Vec<(uuid::Uuid, uuid::Uuid, String)> {
    let mut rows: Vec<_> =
        sqlx::query!("SELECT course_id, user_id, role FROM teaching.memberships")
            .fetch_all(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| (r.course_id, r.user_id, r.role))
            .collect();
    rows.sort();
    rows
}

#[tokio::test]
async fn without_links_effective_members_are_exactly_the_memberships() {
    db_test(async |pool| {
        let exists = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM information_schema.views
               WHERE table_schema = 'teaching' AND table_name = 'effective_members'"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(exists, 1, "a view, not a table");
        assert!(effective_members(&pool).await.is_empty());

        let teacher = owner(&pool, "t@x.edu").await;
        let assistant = owner(&pool, "a@x.edu").await;
        let student = owner(&pool, "s@x.edu").await;
        let other_student = owner(&pool, "o@x.edu").await;
        let course = insert_course(&pool, teacher).await;
        let other_course = insert_course(&pool, teacher).await;
        let add = |course: uuid::Uuid, user: uuid::Uuid, role: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "INSERT INTO teaching.memberships (course_id, user_id, role) VALUES ($1, $2, $3)",
                    course,
                    user,
                    role
                )
                .execute(&pool)
                .await
                .unwrap();
            }
        };
        add(course, teacher, "teacher").await;
        add(course, assistant, "assistant").await;
        add(course, student, "student").await;
        add(other_course, student, "student").await;
        add(other_course, other_student, "student").await;

        let seen = effective_members(&pool).await;
        assert_eq!(seen.len(), 5, "every role, every course");
        assert_eq!(seen, memberships(&pool).await);
        let sources = sqlx::query_scalar!(
            r#"SELECT DISTINCT source AS "source!" FROM teaching.effective_members"#
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(sources, ["direct"]);

        // It follows the table: a role change and a removal show at once.
        sqlx::query!(
            "UPDATE teaching.memberships SET role = 'assistant' WHERE course_id = $1 AND user_id = $2",
            other_course,
            other_student
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "DELETE FROM teaching.memberships WHERE course_id = $1 AND user_id = $2",
            course,
            student
        )
        .execute(&pool)
        .await
        .unwrap();
        let seen = effective_members(&pool).await;
        assert_eq!(seen.len(), 4);
        assert_eq!(seen, memberships(&pool).await);
        assert!(seen.contains(&(other_course, other_student, "assistant".to_owned())));
    })
    .await;
}

// --- migration 14: terms, groups and subgroups (ADR-0032) -----------------------------------------

async fn exec(pool: &PgPool, sql: &'static str, binds: &[uuid::Uuid]) -> Result<(), sqlx::Error> {
    let mut query = sqlx::query(sql);
    for id in binds {
        query = query.bind(id);
    }
    query.execute(pool).await.map(|_| ())
}

async fn count(pool: &PgPool, table: &'static str) -> i64 {
    match table {
        "group_members" => {
            sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM org.group_members"#)
                .fetch_one(pool)
                .await
        }
        "subgroup_members" => {
            sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM org.subgroup_members"#)
                .fetch_one(pool)
                .await
        }
        "subgroups" => {
            sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM org.subgroups"#)
                .fetch_one(pool)
                .await
        }
        _ => panic!("no such table in this helper: {table}"),
    }
    .unwrap()
}

async fn student(pool: &PgPool, email: &str) -> uuid::Uuid {
    insert_user(pool, email, "student").await.unwrap();
    sqlx::query_scalar!("SELECT id FROM auth.users WHERE email = $1", email)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn insert_term(
    pool: &PgPool,
    name: &str,
    starts: &str,
    ends: &str,
) -> Result<uuid::Uuid, sqlx::Error> {
    let id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO org.terms (id, name, starts_on, ends_on) VALUES ($1, $2, $3::text::date, $4::text::date)",
        id,
        name,
        starts,
        ends
    )
    .execute(pool)
    .await?;
    Ok(id)
}

async fn insert_group(pool: &PgPool, name: &str) -> Result<uuid::Uuid, sqlx::Error> {
    let id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO org.groups (id, name) VALUES ($1, $2)",
        id,
        name
    )
    .execute(pool)
    .await?;
    Ok(id)
}

async fn insert_subgroup(
    pool: &PgPool,
    group: uuid::Uuid,
    name: &str,
) -> Result<uuid::Uuid, sqlx::Error> {
    let id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO org.subgroups (id, group_id, name) VALUES ($1, $2, $3)",
        id,
        group,
        name
    )
    .execute(pool)
    .await?;
    Ok(id)
}

#[tokio::test]
async fn a_term_has_a_trimmed_unique_name_and_dates_in_order() {
    db_test(async |pool| {
        insert_term(&pool, "Fall 2026", "2026-09-01", "2027-01-31")
            .await
            .unwrap();
        insert_term(&pool, "One day", "2026-09-01", "2026-09-01")
            .await
            .unwrap();

        assert_eq!(
            sqlstate(insert_term(&pool, "Fall 2026", "2026-09-01", "2027-01-31").await),
            "23505"
        );
        assert_eq!(
            sqlstate(insert_term(&pool, "Backwards", "2027-01-31", "2026-09-01").await),
            "23514"
        );
        for name in ["", " padded", "padded ", &"x".repeat(101)] {
            assert_eq!(
                sqlstate(insert_term(&pool, name, "2026-09-01", "2027-01-31").await),
                "23514",
                "{name:?}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_course_term_must_exist_and_a_used_term_cannot_be_deleted() {
    db_test(async |pool| {
        let teacher = owner(&pool, "t@x.edu").await;
        let term = insert_term(&pool, "Fall 2026", "2026-09-01", "2027-01-31")
            .await
            .unwrap();
        let course = insert_course(&pool, teacher).await;

        let unknown = exec(
            &pool,
            "UPDATE teaching.courses SET term_id = $1 WHERE id = $2",
            &[uuid::Uuid::now_v7(), course],
        )
        .await;
        assert_eq!(sqlstate(unknown), "23503");

        exec(
            &pool,
            "UPDATE teaching.courses SET term_id = $1 WHERE id = $2",
            &[term, course],
        )
        .await
        .unwrap();
        // ON DELETE RESTRICT reports restrict_violation.
        assert_eq!(
            sqlstate(exec(&pool, "DELETE FROM org.terms WHERE id = $1", &[term]).await),
            "23001"
        );

        exec(
            &pool,
            "DELETE FROM teaching.courses WHERE id = $1",
            &[course],
        )
        .await
        .unwrap();
        exec(&pool, "DELETE FROM org.terms WHERE id = $1", &[term])
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn group_and_subgroup_names_are_trimmed_and_unique_where_they_should_be() {
    db_test(async |pool| {
        let group = insert_group(&pool, "7-A").await.unwrap();
        let other = insert_group(&pool, "7-B").await.unwrap();
        assert_eq!(sqlstate(insert_group(&pool, "7-A").await), "23505");
        assert_eq!(sqlstate(insert_group(&pool, " 7-C").await), "23514");
        assert_eq!(sqlstate(insert_group(&pool, "").await), "23514");

        insert_subgroup(&pool, group, "English 1").await.unwrap();
        insert_subgroup(&pool, other, "English 1").await.unwrap();
        assert_eq!(
            sqlstate(insert_subgroup(&pool, group, "English 1").await),
            "23505"
        );
        assert_eq!(
            sqlstate(insert_subgroup(&pool, group, "English 2 ").await),
            "23514"
        );
        assert_eq!(
            sqlstate(insert_subgroup(&pool, uuid::Uuid::now_v7(), "Orphan").await),
            "23503"
        );
    })
    .await;
}

#[tokio::test]
async fn a_subgroup_member_must_be_a_member_of_the_group_and_leaves_with_them() {
    db_test(async |pool| {
        let group = insert_group(&pool, "7-A").await.unwrap();
        let sub = insert_subgroup(&pool, group, "English 1").await.unwrap();
        let sub_too = insert_subgroup(&pool, group, "English 2").await.unwrap();
        let (a, b) = (
            student(&pool, "a@x.edu").await,
            student(&pool, "b@x.edu").await,
        );
        let add_to_group = "INSERT INTO org.group_members (group_id, user_id) VALUES ($1, $2)";
        let add_to_sub =
            "INSERT INTO org.subgroup_members (subgroup_id, group_id, user_id) VALUES ($1, $2, $3)";

        // Not in the group yet: the composite key refuses.
        assert_eq!(
            sqlstate(exec(&pool, add_to_sub, &[sub, group, a]).await),
            "23503"
        );
        exec(&pool, add_to_group, &[group, a]).await.unwrap();
        exec(&pool, add_to_group, &[group, b]).await.unwrap();
        exec(&pool, add_to_sub, &[sub, group, a]).await.unwrap();
        exec(&pool, add_to_sub, &[sub_too, group, a]).await.unwrap();
        exec(&pool, add_to_sub, &[sub, group, b]).await.unwrap();
        // A subgroup row must name the group the subgroup belongs to.
        let other = insert_group(&pool, "7-B").await.unwrap();
        let c = student(&pool, "c@x.edu").await;
        exec(&pool, add_to_group, &[other, c]).await.unwrap();
        assert_eq!(
            sqlstate(exec(&pool, add_to_sub, &[sub, other, c]).await),
            "23503"
        );
        assert_eq!(count(&pool, "subgroup_members").await, 3);

        exec(
            &pool,
            "DELETE FROM org.group_members WHERE group_id = $1 AND user_id = $2",
            &[group, a],
        )
        .await
        .unwrap();
        assert_eq!(
            count(&pool, "subgroup_members").await,
            1,
            "only b is left, in one subgroup"
        );
    })
    .await;
}

#[tokio::test]
async fn deleting_a_group_deletes_its_subgroups_and_members() {
    db_test(async |pool| {
        let group = insert_group(&pool, "7-A").await.unwrap();
        let sub = insert_subgroup(&pool, group, "English 1").await.unwrap();
        let a = student(&pool, "a@x.edu").await;
        exec(
            &pool,
            "INSERT INTO org.group_members (group_id, user_id) VALUES ($1, $2)",
            &[group, a],
        )
        .await
        .unwrap();
        exec(
            &pool,
            "INSERT INTO org.subgroup_members (subgroup_id, group_id, user_id) VALUES ($1, $2, $3)",
            &[sub, group, a],
        )
        .await
        .unwrap();

        exec(&pool, "DELETE FROM org.groups WHERE id = $1", &[group])
            .await
            .unwrap();

        assert_eq!(count(&pool, "group_members").await, 0);
        assert_eq!(count(&pool, "subgroups").await, 0);
        assert_eq!(count(&pool, "subgroup_members").await, 0);
    })
    .await;
}

// --- migration 15: group links, exclusions and the live roster (ADR-0033) -----------------------

async fn link(
    pool: &PgPool,
    course: uuid::Uuid,
    group: uuid::Uuid,
    subgroup: Option<uuid::Uuid>,
    by: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO teaching.course_groups (id, course_id, group_id, subgroup_id, created_by)
         VALUES ($1, $2, $3, $4, $5)",
        uuid::Uuid::now_v7(),
        course,
        group,
        subgroup,
        by
    )
    .execute(pool)
    .await
    .map(|_| ())
}

async fn join_group(pool: &PgPool, group: uuid::Uuid, user: uuid::Uuid) {
    sqlx::query!(
        "INSERT INTO org.group_members (group_id, user_id) VALUES ($1, $2)",
        group,
        user
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn join_subgroup(pool: &PgPool, subgroup: uuid::Uuid, group: uuid::Uuid, user: uuid::Uuid) {
    sqlx::query!(
        "INSERT INTO org.subgroup_members (subgroup_id, group_id, user_id) VALUES ($1, $2, $3)",
        subgroup,
        group,
        user
    )
    .execute(pool)
    .await
    .unwrap();
}

/// (user, role, source) of the course, by user id.
async fn roster(pool: &PgPool, course: uuid::Uuid) -> Vec<(uuid::Uuid, String, String)> {
    // A view's columns are inferred as nullable; all three are not null in its definition.
    sqlx::query!(
        r#"SELECT user_id AS "user_id!", role AS "role!", source AS "source!"
           FROM teaching.effective_members WHERE course_id = $1 ORDER BY user_id"#,
        course
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| (row.user_id, row.role, row.source))
    .collect()
}

fn on(list: &[(uuid::Uuid, String, String)], user: uuid::Uuid) -> Option<(&str, &str)> {
    list.iter()
        .find(|(id, _, _)| *id == user)
        .map(|(_, role, source)| (role.as_str(), source.as_str()))
}

#[tokio::test]
async fn the_students_of_linked_groups_are_on_the_roster_once() {
    db_test(async |pool| {
        let teacher = owner(&pool, "t@x.edu").await;
        let (a, b, c) = (
            student(&pool, "a@x.edu").await,
            student(&pool, "b@x.edu").await,
            student(&pool, "c@x.edu").await,
        );
        let group = insert_group(&pool, "7-A").await.unwrap();
        let sub = insert_subgroup(&pool, group, "English 1").await.unwrap();
        for user in [a, b, c] {
            join_group(&pool, group, user).await;
        }
        join_subgroup(&pool, sub, group, a).await;
        let whole = insert_course(&pool, teacher).await;
        let part = insert_course(&pool, teacher).await;
        let both = insert_course(&pool, teacher).await;
        link(&pool, whole, group, None, teacher).await.unwrap();
        link(&pool, part, group, Some(sub), teacher).await.unwrap();
        link(&pool, both, group, None, teacher).await.unwrap();
        link(&pool, both, group, Some(sub), teacher).await.unwrap();

        let all: Vec<_> = roster(&pool, whole).await;
        assert_eq!(all.len(), 3);
        assert!(
            all.iter()
                .all(|(_, role, source)| role == "student" && source == "group")
        );
        // A subgroup link brings only that subgroup.
        let only: Vec<_> = roster(&pool, part)
            .await
            .into_iter()
            .map(|(id, _, _)| id)
            .collect();
        assert_eq!(only, [a]);
        // A whole-group link and a link to one of its subgroups: still one row per user.
        assert_eq!(roster(&pool, both).await.len(), 3);

        // Leaving the group takes the student off every linked course at once.
        exec(
            &pool,
            "DELETE FROM org.group_members WHERE group_id = $1 AND user_id = $2",
            &[group, a],
        )
        .await
        .unwrap();
        assert_eq!(on(&roster(&pool, whole).await, a), None);
        assert!(roster(&pool, part).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_direct_membership_wins_over_an_exclusion_and_an_exclusion_over_a_group() {
    db_test(async |pool| {
        let teacher = owner(&pool, "t@x.edu").await;
        let (a, b, c) = (
            student(&pool, "a@x.edu").await,
            student(&pool, "b@x.edu").await,
            student(&pool, "c@x.edu").await,
        );
        let group = insert_group(&pool, "7-A").await.unwrap();
        for user in [a, b, c] {
            join_group(&pool, group, user).await;
        }
        let course = insert_course(&pool, teacher).await;
        link(&pool, course, group, None, teacher).await.unwrap();
        // a: a direct assistant and a group student is an assistant. b: excluded. c: nothing special.
        exec(
            &pool,
            "INSERT INTO teaching.memberships (course_id, user_id, role) VALUES ($1, $2, 'assistant')",
            &[course, a],
        )
        .await
        .unwrap();
        let exclude = "INSERT INTO teaching.course_exclusions (course_id, user_id, created_by) VALUES ($1, $2, $3)";
        exec(&pool, exclude, &[course, b, teacher]).await.unwrap();

        let seen = roster(&pool, course).await;
        assert_eq!(on(&seen, a), Some(("assistant", "direct")));
        assert_eq!(on(&seen, b), None);
        assert_eq!(on(&seen, c), Some(("student", "group")));

        // A direct membership beats an exclusion that is still there.
        exec(
            &pool,
            "INSERT INTO teaching.memberships (course_id, user_id, role) VALUES ($1, $2, 'student')",
            &[course, b],
        )
        .await
        .unwrap();
        assert_eq!(
            on(&roster(&pool, course).await, b),
            Some(("student", "direct"))
        );

        // An exclusion of a user who left the group is inert, and one row per (course, user) holds.
        exec(
            &pool,
            "DELETE FROM org.group_members WHERE group_id = $1 AND user_id = $2",
            &[group, c],
        )
        .await
        .unwrap();
        exec(&pool, exclude, &[course, c, teacher]).await.unwrap();
        assert_eq!(on(&roster(&pool, course).await, c), None);
        let rows = roster(&pool, course).await;
        let mut ids: Vec<_> = rows.iter().map(|(id, _, _)| *id).collect();
        ids.dedup();
        assert_eq!(ids.len(), rows.len());
    })
    .await;
}

#[tokio::test]
async fn a_link_names_a_real_group_and_one_of_its_subgroups_and_is_unique() {
    db_test(async |pool| {
        let teacher = owner(&pool, "t@x.edu").await;
        let group = insert_group(&pool, "7-A").await.unwrap();
        let other = insert_group(&pool, "7-B").await.unwrap();
        let sub = insert_subgroup(&pool, group, "English 1").await.unwrap();
        let course = insert_course(&pool, teacher).await;

        assert_eq!(
            sqlstate(link(&pool, course, uuid::Uuid::now_v7(), None, teacher).await),
            "23503"
        );
        // A subgroup of another group, and a subgroup that does not exist.
        assert_eq!(
            sqlstate(link(&pool, course, other, Some(sub), teacher).await),
            "23503"
        );
        assert_eq!(
            sqlstate(link(&pool, course, group, Some(uuid::Uuid::now_v7()), teacher).await),
            "23503"
        );

        link(&pool, course, group, None, teacher).await.unwrap();
        link(&pool, course, group, Some(sub), teacher)
            .await
            .unwrap();
        // Null subgroups count as equal: the same whole-group link twice is refused.
        assert_eq!(
            sqlstate(link(&pool, course, group, None, teacher).await),
            "23505"
        );
        assert_eq!(
            sqlstate(link(&pool, course, group, Some(sub), teacher).await),
            "23505"
        );
    })
    .await;
}

#[tokio::test]
async fn a_linked_group_or_subgroup_is_protected_and_a_deleted_course_takes_its_links() {
    db_test(async |pool| {
        let teacher = owner(&pool, "t@x.edu").await;
        let group = insert_group(&pool, "7-A").await.unwrap();
        let sub = insert_subgroup(&pool, group, "English 1").await.unwrap();
        let course = insert_course(&pool, teacher).await;
        link(&pool, course, group, Some(sub), teacher)
            .await
            .unwrap();

        // ON DELETE RESTRICT reports restrict_violation. A subgroup link also protects its group:
        // the delete would cascade to the linked subgroup.
        assert_eq!(
            sqlstate(exec(&pool, "DELETE FROM org.subgroups WHERE id = $1", &[sub]).await),
            "23001"
        );
        assert_eq!(
            sqlstate(exec(&pool, "DELETE FROM org.groups WHERE id = $1", &[group]).await),
            "23001"
        );

        exec(
            &pool,
            "DELETE FROM teaching.courses WHERE id = $1",
            &[course],
        )
        .await
        .unwrap();
        let links = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM teaching.course_groups"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(links, 0);
        exec(&pool, "DELETE FROM org.groups WHERE id = $1", &[group])
            .await
            .unwrap();
    })
    .await;
}

// ADR-0045, ADR-0046.

#[tokio::test]
async fn migration_21_gives_existing_outbox_rows_their_worker_columns() {
    bare_db_test(async |pool| {
        db::MIGRATOR.run_to(20, &pool).await.unwrap();
        for processed in [true, false] {
            sqlx::query(
                "INSERT INTO events.outbox (id, type, payload, processed_at)
                 VALUES ($1, 'grade.published', '{}', CASE WHEN $2 THEN now() END)",
            )
            .bind(uuid::Uuid::now_v7())
            .bind(processed)
            .execute(&pool)
            .await
            .unwrap();
        }

        db::MIGRATOR.run(&pool).await.unwrap();

        let rows = sqlx::query!(
            r#"SELECT attempts, failed_at IS NULL AS "not_failed!", last_error,
                      run_after <= now() AS "due!", processed_at IS NOT NULL AS "processed!"
               FROM events.outbox ORDER BY id"#
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|r| r.attempts == 0 && r.not_failed && r.last_error.is_none() && r.due)
        );
        assert_eq!(rows.iter().filter(|r| r.processed).count(), 1);
        db::ensure_migrated(&pool).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_processed_outbox_row_is_not_also_failed() {
    db_test(async |pool| {
        let id = uuid::Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO events.outbox (id, type, payload) VALUES ($1, 'grade.published', '{}')",
            id
        )
        .execute(&pool)
        .await
        .unwrap();

        let both = sqlx::query!(
            "UPDATE events.outbox SET processed_at = now(), failed_at = now() WHERE id = $1",
            id
        )
        .execute(&pool)
        .await;
        assert_eq!(sqlstate(both), "23514");
        let negative = sqlx::query!("UPDATE events.outbox SET attempts = -1 WHERE id = $1", id)
            .execute(&pool)
            .await;
        assert_eq!(sqlstate(negative), "23514");
    })
    .await;
}

#[tokio::test]
async fn migration_22_counts_every_existing_account_as_verified() {
    bare_db_test(async |pool| {
        db::MIGRATOR.run_to(21, &pool).await.unwrap();
        for email in ["a@x.edu", "b@x.edu"] {
            insert_user_before_auth(&pool, email, "student").await;
        }
        sqlx::query("UPDATE identity.users SET email_verified_at = NULL")
            .execute(&pool)
            .await
            .unwrap();

        db::MIGRATOR.run(&pool).await.unwrap();

        let unverified = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM auth.users
               WHERE email_verified_at IS DISTINCT FROM created_at"#
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unverified, 0);
        db::ensure_migrated(&pool).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn migration_32_renames_the_identity_schema_and_keeps_its_rows() {
    bare_db_test(async |pool| {
        db::MIGRATOR.run_to(31, &pool).await.unwrap();
        let user = insert_user_before_auth(&pool, "a@x.edu", "teacher").await;

        db::MIGRATOR.run(&pool).await.unwrap();

        let schemas = sqlx::query_scalar!(
            r#"SELECT schema_name AS "name!" FROM information_schema.schemata
               WHERE schema_name IN ('identity', 'auth')"#
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(schemas, ["auth"]);
        let email = sqlx::query_scalar!("SELECT email FROM auth.users WHERE id = $1", user)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(email, "a@x.edu");
        db::ensure_migrated(&pool).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn email_tokens_are_hashed_unique_and_expire_after_they_are_made() {
    db_test(async |pool| {
        insert_user(&pool, "a@x.edu", "student").await.unwrap();
        let user = sqlx::query_scalar!("SELECT id FROM auth.users")
            .fetch_one(&pool)
            .await
            .unwrap();
        let insert = |purpose: &'static str, hash: Vec<u8>, hours: i32| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "INSERT INTO auth.email_tokens (id, user_id, purpose, token_hash, expires_at)
                     VALUES ($1, $2, $3, $4, now() + make_interval(hours => $5))",
                    uuid::Uuid::now_v7(),
                    user,
                    purpose,
                    hash,
                    hours
                )
                .execute(&pool)
                .await
            }
        };

        insert("verify", vec![1; 32], 24).await.unwrap();
        assert_eq!(
            sqlstate(insert("verify", vec![1; 32], 24).await),
            "23505",
            "unique"
        );
        assert_eq!(
            sqlstate(insert("other", vec![2; 32], 24).await),
            "23514",
            "purpose"
        );
        assert_eq!(
            sqlstate(insert("verify", vec![3; 31], 24).await),
            "23514",
            "a sha-256"
        );
        assert_eq!(
            sqlstate(insert("reset", vec![4; 32], 0).await),
            "23514",
            "expires later"
        );

        sqlx::query!("DELETE FROM auth.users WHERE id = $1", user)
            .execute(&pool)
            .await
            .unwrap();
        let left = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM auth.email_tokens"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "tokens go with their user");
    })
    .await;
}

// ADR-0047.

#[tokio::test]
async fn a_google_sign_in_attempt_is_found_by_two_hashes_and_expires_after_it_starts() {
    db_test(async |pool| {
        let insert = |state: Vec<u8>, browser: Vec<u8>, minutes: i32| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "INSERT INTO auth.oauth_states
                         (state_hash, browser_hash, nonce, pkce_verifier, expires_at)
                     VALUES ($1, $2, 'n', 'v', now() + make_interval(mins => $3))",
                    state,
                    browser,
                    minutes
                )
                .execute(&pool)
                .await
            }
        };

        insert(vec![1; 32], vec![2; 32], 10).await.unwrap();
        assert_eq!(
            sqlstate(insert(vec![1; 32], vec![3; 32], 10).await),
            "23505",
            "one row per state"
        );
        assert_eq!(
            sqlstate(insert(vec![4; 31], vec![2; 32], 10).await),
            "23514",
            "a sha-256"
        );
        assert_eq!(
            sqlstate(insert(vec![5; 32], vec![2; 5], 10).await),
            "23514",
            "a sha-256"
        );
        assert_eq!(
            sqlstate(insert(vec![6; 32], vec![2; 32], 0).await),
            "23514",
            "expires later"
        );
    })
    .await;
}

// ADR-0048.

#[tokio::test]
async fn a_course_has_an_empty_code_and_description_until_they_are_set_and_both_are_bounded() {
    db_test(async |pool| {
        let teacher = owner(&pool, "t@x.edu").await;
        let course = insert_course(&pool, teacher).await;
        let row = sqlx::query!(
            "SELECT code, description FROM teaching.courses WHERE id = $1",
            course
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((row.code.as_str(), row.description.as_str()), ("", ""));

        let set = |code: String, description: String| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "UPDATE teaching.courses SET code = $2, description = $3 WHERE id = $1",
                    course,
                    code,
                    description
                )
                .execute(&pool)
                .await
            }
        };
        set("CS 305".into(), "x".repeat(300)).await.unwrap();
        assert_eq!(
            sqlstate(set("c".repeat(33), String::new()).await),
            "23514",
            "code length"
        );
        assert_eq!(
            sqlstate(set(" CS".into(), String::new()).await),
            "23514",
            "code is trimmed"
        );
        assert_eq!(
            sqlstate(set(String::new(), "y".repeat(301)).await),
            "23514",
            "description length"
        );
        assert_eq!(
            sqlstate(set(String::new(), " d".into()).await),
            "23514",
            "description is trimmed"
        );
    })
    .await;
}

#[tokio::test]
async fn group_roster_links_has_a_row_per_link_where_the_roster_has_one_per_student() {
    db_test(async |pool| {
        let teacher = owner(&pool, "t@x.edu").await;
        let a = student(&pool, "a@x.edu").await;
        let group = insert_group(&pool, "7-A").await.unwrap();
        let sub = insert_subgroup(&pool, group, "English 1").await.unwrap();
        join_group(&pool, group, a).await;
        join_subgroup(&pool, sub, group, a).await;
        let course = insert_course(&pool, teacher).await;
        link(&pool, course, group, None, teacher).await.unwrap();
        link(&pool, course, group, Some(sub), teacher)
            .await
            .unwrap();

        let rows = sqlx::query_scalar!(
            r#"SELECT count(DISTINCT link_id) AS "n!" FROM teaching.group_roster_links
               WHERE course_id = $1 AND user_id = $2"#,
            course,
            a
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, 2, "one row per link");
        assert_eq!(
            roster(&pool, course).await.len(),
            1,
            "still one student on the roster"
        );
    })
    .await;
}

// ADR-0049.

#[tokio::test]
async fn migration_26_gives_published_items_a_published_copy_and_leaves_drafts_without_one() {
    bare_db_test(async |pool| {
        db::MIGRATOR.run_to(25, &pool).await.unwrap();
        let teacher = insert_user_before_auth(&pool, "t@x.edu", "teacher").await;
        let course = insert_course(&pool, teacher).await;
        let module = insert_module_row(&pool, course, None, 1).await.unwrap();
        let live = insert_item_of_kind(&pool, module, course, 1, "text")
            .await
            .unwrap();
        let draft = insert_item_of_kind(&pool, module, course, 2, "text")
            .await
            .unwrap();
        sqlx::query!(
            "UPDATE content.items SET status = 'published', title = 'Live', body = $2 WHERE id = $1",
            live,
            serde_json::json!({"v": 1})
        )
        .execute(&pool)
        .await
        .unwrap();

        db::MIGRATOR.run(&pool).await.unwrap();

        let row = |id: uuid::Uuid| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    r#"SELECT published_title, published_body, last_published_at IS NOT NULL AS "stamped!"
                       FROM content.items WHERE id = $1"#,
                    id
                )
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        let published = row(live).await;
        assert_eq!(published.published_title.as_deref(), Some("Live"));
        assert_eq!(published.published_body, Some(serde_json::json!({"v": 1})));
        assert!(published.stamped);
        let waiting = row(draft).await;
        assert!(waiting.published_title.is_none() && waiting.published_body.is_none());
        assert!(!waiting.stamped);
        db::ensure_migrated(&pool).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn a_published_item_needs_its_published_copy_and_only_in_class_work_has_a_class_day() {
    db_test(async |pool| {
        let teacher = owner(&pool, "t@x.edu").await;
        let course = insert_course(&pool, teacher).await;
        let module = insert_module_row(&pool, course, None, 1).await.unwrap();
        let item = insert_item_of_kind(&pool, module, course, 1, "assignment")
            .await
            .unwrap();

        let publish = sqlx::query!(
            "UPDATE content.items SET status = 'published' WHERE id = $1",
            item
        )
        .execute(&pool)
        .await;
        assert_eq!(sqlstate(publish), "23514", "published needs a copy");
        sqlx::query!(
            "UPDATE content.items
             SET status = 'published', published_title = title, published_body = body,
                 last_published_at = now()
             WHERE id = $1",
            item
        )
        .execute(&pool)
        .await
        .unwrap();

        insert_assignment_row(&pool, item, "10", serde_json::json!({}))
            .await
            .unwrap();
        let on_text = sqlx::query!(
            "UPDATE content.assignments SET class_on = '2026-10-29' WHERE item_id = $1",
            item
        )
        .execute(&pool)
        .await;
        assert_eq!(
            sqlstate(on_text),
            "23514",
            "a text assignment has no class day"
        );
        sqlx::query!(
            "UPDATE content.assignments SET type = 'no_submission', class_on = '2026-10-29'
             WHERE item_id = $1",
            item
        )
        .execute(&pool)
        .await
        .unwrap();
    })
    .await;
}

// --- migration 27: quiz scores and grades without a person ---------------------------------------

#[tokio::test]
async fn only_a_submitted_attempt_has_scores_and_only_an_auto_score_has_no_person() {
    db_test(async |pool| {
        let student = owner(&pool, "s@x.edu").await;
        let assignment = uuid::Uuid::now_v7();

        let insert = |status: &'static str, scores: Option<serde_json::Value>| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "INSERT INTO assessment.submissions
                         (id, assignment_id, student_id, status, attempt, content, submitted_at,
                          auto_scores)
                     VALUES ($1, $2, $3, $4, CASE WHEN $4 = 'submitted' THEN 1 END, '{}',
                             CASE WHEN $4 = 'submitted' THEN now() END, $5)",
                    uuid::Uuid::now_v7(),
                    assignment,
                    student,
                    status,
                    scores
                )
                .execute(&pool)
                .await
            }
        };
        let scores = serde_json::json!({"q1": "2.00"});
        assert_eq!(
            sqlstate(insert("draft", Some(scores.clone())).await),
            "23514",
            "a draft has no scores"
        );
        assert_eq!(
            sqlstate(insert("submitted", Some(serde_json::json!(["x"]))).await),
            "23514",
            "the scores are an object"
        );
        insert("submitted", Some(scores)).await.unwrap();

        // A draft grade the server wrote has no person; a history row has none only for `auto_score`.
        sqlx::query!(
            "INSERT INTO gradebook.grades (assignment_id, student_id, draft_score, draft_updated_by)
             VALUES ($1, $2, 3, NULL)",
            assignment,
            student
        )
        .execute(&pool)
        .await
        .unwrap();
        let history = |action: &'static str, actor: Option<uuid::Uuid>| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "INSERT INTO gradebook.grade_history
                         (id, assignment_id, student_id, action, actor_id, score)
                     VALUES ($1, $2, $3, $4, $5, 3)",
                    uuid::Uuid::now_v7(),
                    assignment,
                    student,
                    action,
                    actor
                )
                .execute(&pool)
                .await
            }
        };
        history("auto_score", None).await.unwrap();
        history("save_draft", Some(student)).await.unwrap();
        history("publish", Some(student)).await.unwrap();
        for (action, actor) in [
            ("auto_score", Some(student)),
            ("save_draft", None),
            ("publish", None),
        ] {
            assert_eq!(sqlstate(history(action, actor).await), "23514", "{action}");
        }
    })
    .await;
}

// --- migration 28: sign-in times and when a person joined a group --------------------------------

#[tokio::test]
async fn a_sign_in_time_is_one_row_per_person_and_goes_with_the_person() {
    db_test(async |pool| {
        let user = owner(&pool, "s@x.edu").await;
        let record = |at: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query!(
                    "INSERT INTO auth.last_logins (user_id, at) VALUES ($1, $2::text::timestamptz)",
                    user,
                    at
                )
                .execute(&pool)
                .await
            }
        };
        record("2026-10-01T10:00:00Z").await.unwrap();
        assert_eq!(sqlstate(record("2026-10-02T10:00:00Z").await), "23505");
        let unknown = sqlx::query!(
            "INSERT INTO auth.last_logins (user_id, at) VALUES ($1, now())",
            uuid::Uuid::now_v7()
        )
        .execute(&pool)
        .await;
        assert_eq!(sqlstate(unknown), "23503", "only a real person signs in");
    })
    .await;
}

#[tokio::test]
async fn a_group_member_is_stamped_when_added_and_older_rows_have_no_time() {
    db_test(async |pool| {
        let user = owner(&pool, "s@x.edu").await;
        let group = uuid::Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO org.groups (id, name) VALUES ($1, 'Group A')",
            group
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query!(
            "INSERT INTO org.group_members (group_id, user_id) VALUES ($1, $2)",
            group,
            user
        )
        .execute(&pool)
        .await
        .unwrap();
        let stamped = sqlx::query_scalar!(
            r#"SELECT added_at IS NOT NULL AS "stamped!" FROM org.group_members WHERE group_id = $1"#,
            group
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(stamped, "the default is now()");
        sqlx::query!("UPDATE org.group_members SET added_at = NULL WHERE group_id = $1", group)
            .execute(&pool)
            .await
            .unwrap();
    })
    .await;
}
