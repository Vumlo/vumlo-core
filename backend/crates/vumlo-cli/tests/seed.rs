//! `vumlo-cli seed --demo`: idempotent, careful with a database that is not empty, audited, and
//! what it writes works through the real server.

mod common;

use common::{Env, audit, db_test, migrate_modules, settings_for, spawn_server};
use serde_json::{Value, json};
use sqlx::PgPool;

async fn count(pool: &PgPool, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap()
}

/// `(email, password)` of each account the seed printed.
fn accounts(stdout: &str) -> Vec<(String, String)> {
    stdout
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let role = words.next()?;
            let email = words.next()?;
            let password = words.next()?;
            (["teacher", "student"].contains(&role) && email.contains('@'))
                .then(|| (email.to_owned(), password.to_owned()))
        })
        .collect()
}

async fn call(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    token: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut request = client.request(method, url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    let text = response.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[tokio::test]
async fn it_creates_the_demo_data_once_and_audits_it() {
    db_test(async |pool| {
        migrate_modules(&pool).await;
        let env = Env::new(&pool);

        let dry = env.run(&["seed", "--demo", "--dry-run"]).await;
        assert!(
            dry.ok && dry.stdout.contains("would create"),
            "{}",
            dry.stderr
        );
        assert_eq!(
            count(&pool, "SELECT count(*) FROM teaching.courses").await,
            0
        );

        let first = env.run(&["seed", "--demo"]).await;
        assert!(first.ok, "{}", first.stderr);
        assert_eq!(accounts(&first.stdout).len(), 4, "{}", first.stdout);
        assert_eq!(
            count(&pool, "SELECT count(*) FROM teaching.courses").await,
            1
        );
        assert_eq!(count(&pool, "SELECT count(*) FROM auth.users").await, 4);
        assert_eq!(count(&pool, "SELECT count(*) FROM content.items").await, 2);
        assert_eq!(
            count(&pool, "SELECT count(*) FROM gradebook.grades").await,
            3
        );
        assert_eq!(
            audit(&pool).await,
            [("demo.seed".into(), "cli".into(), true, "course".into())]
        );

        let again = env.run(&["seed", "--demo"]).await;
        assert!(
            again.ok && again.stdout.contains("already there"),
            "{}",
            again.stdout
        );
        assert_eq!(count(&pool, "SELECT count(*) FROM auth.users").await, 4);
        assert_eq!(
            audit(&pool).await.len(),
            1,
            "a run that changed nothing writes no row"
        );
    })
    .await;
}

#[tokio::test]
async fn it_asks_for_confirmation_on_a_database_that_is_not_empty() {
    db_test(async |pool| {
        migrate_modules(&pool).await;
        let env = Env::new(&pool);
        sqlx::query(
            "INSERT INTO auth.users (id, email, name, surname, role)
             VALUES (gen_random_uuid(), 'someone@school.edu', 'Some', 'One', 'student')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let refused = env.run(&["seed", "--demo"]).await;
        assert!(!refused.ok);
        assert!(refused.stderr.contains("--yes"), "{}", refused.stderr);
        assert_eq!(
            count(&pool, "SELECT count(*) FROM teaching.courses").await,
            0
        );
        assert!(audit(&pool).await.is_empty());

        let confirmed = env.run(&["seed", "--demo", "--yes"]).await;
        assert!(confirmed.ok, "{}", confirmed.stderr);
        assert_eq!(
            count(&pool, "SELECT count(*) FROM teaching.courses").await,
            1
        );
    })
    .await;
}

#[tokio::test]
async fn a_database_that_only_has_an_admin_counts_as_empty() {
    db_test(async |pool| {
        migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let admin = env
            .run_with(
                &[
                    "admin",
                    "create",
                    "root@school.edu",
                    "--name",
                    "Root",
                    "--surname",
                    "Admin",
                    "--password-stdin",
                ],
                Some("correct horse battery staple\n"),
                &[("DATABASE_URL", env.url.as_str())],
            )
            .await;
        assert!(admin.ok, "{}", admin.stderr);

        let out = env.run(&["seed", "--demo"]).await;

        assert!(out.ok, "{}", out.stderr);
        assert_eq!(audit(&pool).await.len(), 2);
    })
    .await;
}

#[tokio::test]
async fn without_a_target_it_says_what_to_pass() {
    db_test(async |pool| {
        let out = Env::new(&pool).run(&["seed"]).await;
        assert!(!out.ok);
        assert!(out.stderr.contains("--demo"), "{}", out.stderr);
    })
    .await;
}

/// What the seed writes is what the API serves: the students see the course and the quiz without
/// its answer key, their grades are published, and the quiz can be taken and scored.
#[tokio::test]
async fn the_demo_data_works_through_the_real_server() {
    db_test(async |pool| {
        migrate_modules(&pool).await;
        let env = Env::new(&pool);
        let seeded = env.run(&["seed", "--demo"]).await;
        assert!(seeded.ok, "{}", seeded.stderr);
        let people = accounts(&seeded.stdout);
        let base = spawn_server(&pool, settings_for(&env.url, common::CONFIG, &[])).await;
        let api = format!("{base}/api/v1");
        let client = reqwest::Client::new();

        // Signs in with the temporary password, changes it as the account must, signs in again.
        let sign_in = |email: String, temporary: String| {
            let client = client.clone();
            let api = api.clone();
            async move {
                let login = |password: String| {
                    let client = client.clone();
                    let api = api.clone();
                    let email = email.clone();
                    async move {
                        call(
                            &client,
                            reqwest::Method::POST,
                            format!("{api}/auth/login"),
                            None,
                            Some(json!({"email": email, "password": password, "client": "app"})),
                        )
                        .await
                    }
                };
                let (status, body) = login(temporary.clone()).await;
                assert_eq!(status, 200, "{body}");
                let token = body["token"].as_str().unwrap().to_owned();
                let better = "A-much-better-passphrase-7".to_owned();
                let (status, body) = call(
                    &client,
                    reqwest::Method::POST,
                    format!("{api}/me/password"),
                    Some(&token),
                    Some(json!({"current_password": temporary, "new_password": better})),
                )
                .await;
                assert!(status == 200 || status == 204, "{status} {body}");
                let (status, body) = login(better).await;
                assert_eq!(status, 200, "{body}");
                body["token"].as_str().unwrap().to_owned()
            }
        };

        let course: uuid::Uuid = sqlx::query_scalar("SELECT id FROM teaching.courses")
            .fetch_one(&pool)
            .await
            .unwrap();
        let quiz: uuid::Uuid =
            sqlx::query_scalar("SELECT item_id FROM content.assignments WHERE type = 'quiz'")
                .fetch_one(&pool)
                .await
                .unwrap();

        let (olena, password) = people
            .iter()
            .find(|(email, _)| email.starts_with("olena"))
            .cloned()
            .unwrap();
        let token = sign_in(olena, password).await;

        // The course is on her page, the quiz is there and it carries no answer key.
        let (status, content) = call(
            &client,
            reqwest::Method::GET,
            format!("{api}/courses/{course}/content"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{content}");
        assert!(content.to_string().contains("Warm-up quiz"), "{content}");
        let (status, item) = call(
            &client,
            reqwest::Method::GET,
            format!("{api}/items/{quiz}"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{item}");
        let text = item.to_string();
        assert!(text.contains("single_choice"), "{text}");
        assert!(
            !text.contains("\"correct\"") && !text.contains("\"answer\""),
            "{text}"
        );
        assert!(!text.contains("explanation"), "{text}");

        // Her published grade is there.
        let (status, grades) = call(
            &client,
            reqwest::Method::GET,
            format!("{api}/courses/{course}/my-grades"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{grades}");
        assert!(grades.to_string().contains("10"), "{grades}");
        assert!(!grades.to_string().contains("draft_"), "{grades}");

        // The quiz is valid enough to be taken and scored.
        let (status, body) = call(
            &client,
            reqwest::Method::PUT,
            format!("{api}/assignments/{quiz}/submission/draft"),
            Some(&token),
            Some(json!({"content": {"answers": {"q1": ["b"], "q2": "42"}}})),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let (status, body) = call(
            &client,
            reqwest::Method::POST,
            format!("{api}/assignments/{quiz}/submission/submit"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(status, 201, "{body}");

        // And the teacher sees the gradebook.
        let (teacher, password) = people
            .iter()
            .find(|(email, _)| email.starts_with("maria"))
            .cloned()
            .unwrap();
        let teacher_token = sign_in(teacher, password).await;
        let (status, gradebook) = call(
            &client,
            reqwest::Method::GET,
            format!("{api}/courses/{course}/gradebook"),
            Some(&teacher_token),
            None,
        )
        .await;
        assert_eq!(status, 200, "{gradebook}");
    })
    .await;
}
