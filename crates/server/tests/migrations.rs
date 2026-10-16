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
        "INSERT INTO identity.users (id, email, name, surname, role) VALUES ($1, $2, 'Test', 'User', $3)",
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
        "INSERT INTO identity.users (id, email, name, surname, role) VALUES ($1, 'c@x.edu', 'C', 'D', 'teacher')",
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

/// A teacher as the schema before migration 10 stores them: one `display_name`. A runtime query,
/// because the compile-time macros are checked against the full schema, where that column is gone.
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

#[sqlx::test(migrations = false)]
async fn migration_7_turns_lesson_items_into_text_items(pool: PgPool) {
    app::MIGRATOR.run_to(6, &pool).await.unwrap();

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

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn an_item_without_a_body_starts_with_the_empty_document(pool: PgPool) {
    let (_, text) = assignment_fixture(&pool).await;
    let empty: serde_json::Value = serde_json::from_str(EMPTY_DOCUMENT).unwrap();
    assert_eq!(item_body(&pool, text).await, empty);
}

#[sqlx::test(migrations = false)]
async fn migration_8_turns_empty_object_bodies_into_the_empty_document(pool: PgPool) {
    app::MIGRATOR.run_to(7, &pool).await.unwrap();

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

    app::MIGRATOR.run(&pool).await.unwrap();

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
    app::ensure_migrated(&pool).await.unwrap();
}

#[sqlx::test(migrations = false)]
async fn migration_10_splits_display_names_into_name_and_surname(pool: PgPool) {
    app::MIGRATOR.run_to(9, &pool).await.unwrap();

    let cases = [
        ("a@x.edu", "Anna Kowalska", "Anna", "Kowalska"),
        ("b@x.edu", "Anna van der Berg", "Anna", "van der Berg"),
        ("c@x.edu", "Jan \t  Nowak", "Jan", "Nowak"),
        ("d@x.edu", "Madonna", "Madonna", "Madonna"),
    ];
    for (email, display_name, _, _) in cases {
        insert_user_before_names(&pool, email, display_name, "student").await;
    }

    app::MIGRATOR.run(&pool).await.unwrap();

    for (email, _, name, surname) in cases {
        let row = sqlx::query!(
            "SELECT name, surname FROM identity.users WHERE email = $1",
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
           WHERE table_schema = 'identity' AND table_name = 'users' AND column_name = 'display_name'"#
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(column, 0, "display_name is dropped");
    app::ensure_migrated(&pool).await.unwrap();
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_user_needs_a_trimmed_name_and_surname_of_at_most_a_hundred_characters(pool: PgPool) {
    let insert = |name: String, surname: String| {
        let pool = pool.clone();
        async move {
            sqlx::query!(
                "INSERT INTO identity.users (id, email, name, surname, role)
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

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn a_grade_row_checks_scores_and_feedback_but_not_the_assignment(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn published_by_and_published_at_go_together(pool: PgPool) {
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
}

#[sqlx::test(migrator = "server::app::MIGRATOR")]
async fn the_grade_history_is_append_only(pool: PgPool) {
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
}
