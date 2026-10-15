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

    assert!(matches!(error, MigrationCheckError::Pending(1)), "{error}");
    assert!(
        error.to_string().contains("learnly-server migrate"),
        "{error}"
    );

    app::MIGRATOR.run(&pool).await.unwrap();

    app::ensure_migrated(&pool).await.unwrap();
}
