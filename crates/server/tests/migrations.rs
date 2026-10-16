use server::app::{self, MigrationCheckError};
use sqlx::PgPool;

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn migrations_create_exactly_the_module_schemas(pool: PgPool) {
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
            "content",
            "events",
            "files",
            "gradebook",
            "identity",
            "org",
            "teaching"
        ]
    );
}

#[sqlx::test(migrations = false)]
async fn serve_check_refuses_an_unmigrated_database(pool: PgPool) {
    let error = app::ensure_migrated(&pool).await.unwrap_err();

    let embedded = app::MIGRATOR.iter().count();
    assert!(
        matches!(error, MigrationCheckError::Pending(n) if n == embedded),
        "{error}"
    );
    assert!(
        error.to_string().contains("learnly-server migrate"),
        "{error}"
    );

    app::MIGRATOR.run(&pool).await.unwrap();

    app::ensure_migrated(&pool).await.unwrap();
}

async fn insert_user(pool: &PgPool, email: &str, role: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO identity.users (id, email, display_name, role) VALUES ($1, $2, 'Test', $3)",
        uuid::Uuid::now_v7(),
        email,
        role
    )
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn identity_schema_rejects_invalid_users(pool: PgPool) {
    insert_user(&pool, "a@x.edu", "student").await.unwrap();

    assert!(insert_user(&pool, "a@x.edu", "student").await.is_err());
    assert!(
        insert_user(&pool, "A@x.edu", "student").await.is_err(),
        "emails must be stored lowercase"
    );
    assert!(insert_user(&pool, "b@x.edu", "owner").await.is_err());
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn deleting_a_user_cascades_to_identities_and_sessions(pool: PgPool) {
    let user_id = uuid::Uuid::now_v7();
    sqlx::query!(
        "INSERT INTO identity.users (id, email, display_name, role) VALUES ($1, 'c@x.edu', 'C', 'teacher')",
        user_id
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query!(
        "INSERT INTO identity.identities (id, user_id, provider, subject, password_hash)
         VALUES ($1, $2, 'password', $3, 'hash')",
        uuid::Uuid::now_v7(),
        user_id,
        user_id.to_string()
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query!(
        "INSERT INTO identity.sessions (id, user_id, token_hash, kind, expires_at)
         VALUES ($1, $2, $3, 'web', now() + interval '1 day')",
        uuid::Uuid::now_v7(),
        user_id,
        vec![0u8; 32]
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query!("DELETE FROM identity.users WHERE id = $1", user_id)
        .execute(&pool)
        .await
        .unwrap();

    let left = sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM identity.identities)
                + (SELECT count(*) FROM identity.sessions) AS "n!""#
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn password_identity_requires_a_hash_and_google_must_not_have_one(pool: PgPool) {
    insert_user(&pool, "d@x.edu", "student").await.unwrap();
    let user_id = sqlx::query_scalar!(r#"SELECT id AS "id!" FROM identity.users"#)
        .fetch_one(&pool)
        .await
        .unwrap();

    let no_hash = sqlx::query!(
        "INSERT INTO identity.identities (id, user_id, provider, subject)
         VALUES ($1, $2, 'password', 's1')",
        uuid::Uuid::now_v7(),
        user_id
    )
    .execute(&pool)
    .await;
    let google_with_hash = sqlx::query!(
        "INSERT INTO identity.identities (id, user_id, provider, subject, password_hash)
         VALUES ($1, $2, 'google', 's2', 'h')",
        uuid::Uuid::now_v7(),
        user_id
    )
    .execute(&pool)
    .await;

    assert!(no_hash.is_err());
    assert!(google_with_hash.is_err());
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
    sqlx::query_scalar!("SELECT id FROM identity.users WHERE email = $1", email)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn content_modules_keep_one_level_and_one_course(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn content_positions_are_unique_per_sequence_and_checked_at_commit(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn content_items_must_belong_to_their_modules_course(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn content_item_body_must_be_a_json_object(pool: PgPool) {
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

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn item_kind_accepts_assignment_and_rejects_anything_else(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn an_assignment_row_belongs_to_an_assignment_item_only(pool: PgPool) {
    let (assignment, text) = assignment_fixture(&pool).await;

    assert_eq!(
        sqlstate(insert_assignment_row(&pool, text, "5", serde_json::json!({})).await),
        "23503",
        "a text item cannot have an assignment row"
    );
    assert_eq!(
        sqlstate(
            insert_assignment_row(&pool, uuid::Uuid::now_v7(), "5", serde_json::json!({})).await
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn an_item_with_an_assignment_row_cannot_change_kind(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn assignment_points_are_above_zero_and_fit_numeric_6_2(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn assignment_config_must_be_a_json_object(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn assignments_are_not_bonus_by_default_and_go_with_their_item(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn deleting_a_module_removes_its_assignment_rows(pool: PgPool) {
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
}

async fn item_kind(pool: &PgPool, id: uuid::Uuid) -> String {
    sqlx::query_scalar!("SELECT kind FROM content.items WHERE id = $1", id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = false)]
async fn migration_7_turns_lesson_items_into_text_items(pool: PgPool) {
    app::MIGRATOR.run_to(6, &pool).await.unwrap();

    // Data as it exists before the rename: a `lesson` item and an assignment with its row.
    // The macros are checked against the full schema, but only name columns that exist at 6.
    let owner = owner(&pool, "t@x.edu").await;
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

    app::MIGRATOR.run(&pool).await.unwrap();

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
    app::ensure_migrated(&pool).await.unwrap();
}
