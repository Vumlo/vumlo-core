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
