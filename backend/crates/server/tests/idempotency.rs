//! `Idempotency-Key` on `POST` (ADR-0058): a retry gets the answer of the first request, not a second
//! effect. The module of a course is the plain case; a submit is the case a send queue exists for.

mod common;

use common::{Account, Env, Resp, db_test, essay};
use serde_json::{Value, json};
use server::identity::Role;
use sqlx::PgPool;
use uuid::Uuid;

async fn post_with_key(env: &Env, token: &str, path: &str, key: &str, body: Option<Value>) -> Resp {
    let request = env
        .http
        .post(path)
        .bearer(token)
        .header("idempotency-key", key);
    match body {
        Some(body) => request.json(&body).send().await,
        None => request.send().await,
    }
}

async fn teacher_world(pool: PgPool) -> (Env, Account, Uuid) {
    let env = Env::new(pool).await;
    let owner = env.user("owner@x.edu", Role::Teacher).await;
    let (course, _) = env.course(&owner, &[]).await;
    (env, owner, course)
}

/// How many modules of `course` have this title.
async fn modules_titled(env: &Env, token: &str, course: Uuid, title: &str) -> usize {
    let tree = env
        .ok(
            "GET",
            token,
            &format!("/courses/{course}/content"),
            None,
            200,
        )
        .await;
    tree["modules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["title"] == title)
        .count()
}

async fn stored_keys(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM idempotency.keys"#)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_retry_with_the_same_key_gets_the_first_answer_and_does_not_repeat_the_effect() {
    db_test(async |pool| {
        let (env, owner, course) = teacher_world(pool).await;
        let path = format!("/courses/{course}/modules");
        let body = json!({"title": "Once"});
        let first = post_with_key(&env, &owner.token, &path, "k-1", Some(body.clone())).await;
        assert_eq!(first.status, 201);
        assert_eq!(first.headers.get("idempotency-replayed"), None);
        let again = post_with_key(&env, &owner.token, &path, "k-1", Some(body)).await;
        assert_eq!(again.status, 201);
        assert_eq!(again.body, first.body, "the same module, the same id");
        assert_eq!(again.header("idempotency-replayed"), "true");
        assert_eq!(again.header("content-type"), first.header("content-type"));
        assert_eq!(modules_titled(&env, &owner.token, course, "Once").await, 1);
    })
    .await;
}

#[tokio::test]
async fn without_the_header_every_post_is_a_new_request() {
    db_test(async |pool| {
        let (env, owner, course) = teacher_world(pool).await;
        let path = format!("/courses/{course}/modules");
        for _ in 0..2 {
            env.ok(
                "POST",
                &owner.token,
                &path,
                Some(json!({"title": "Twice"})),
                201,
            )
            .await;
        }
        assert_eq!(modules_titled(&env, &owner.token, course, "Twice").await, 2);
        assert_eq!(stored_keys(&env.pool).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_key_used_for_another_request_is_refused() {
    db_test(async |pool| {
        let (env, owner, course) = teacher_world(pool).await;
        let path = format!("/courses/{course}/modules");
        post_with_key(
            &env,
            &owner.token,
            &path,
            "k-2",
            Some(json!({"title": "A"})),
        )
        .await;
        let other_body = post_with_key(
            &env,
            &owner.token,
            &path,
            "k-2",
            Some(json!({"title": "B"})),
        )
        .await;
        assert_eq!(other_body.status, 422);
        assert_eq!(other_body.problem(422).errors[0].field, "Idempotency-Key");
        let other_path = format!("/courses/{course}/publish");
        let response = post_with_key(&env, &owner.token, &other_path, "k-2", None).await;
        assert_eq!(response.status, 422);
        assert_eq!(modules_titled(&env, &owner.token, course, "B").await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_key_belongs_to_the_credential_that_sent_it() {
    db_test(async |pool| {
        let (env, owner, course) = teacher_world(pool).await;
        let second = env.user("second@x.edu", Role::Teacher).await;
        let (course_two, _) = env.course(&second, &[]).await;
        let first = post_with_key(
            &env,
            &owner.token,
            &format!("/courses/{course}/modules"),
            "same",
            Some(json!({"title": "M1"})),
        )
        .await;
        // The same key, another person, another course: not a replay, not a clash.
        let other = post_with_key(
            &env,
            &second.token,
            &format!("/courses/{course_two}/modules"),
            "same",
            Some(json!({"title": "M2"})),
        )
        .await;
        assert_eq!((first.status, other.status), (201, 201));
        assert_eq!(other.headers.get("idempotency-replayed"), None);
        assert_eq!(stored_keys(&env.pool).await, 2);
    })
    .await;
}

#[tokio::test]
async fn two_requests_sent_together_with_one_key_make_one_module() {
    db_test(async |pool| {
        let (env, owner, course) = teacher_world(pool).await;
        let path = format!("/courses/{course}/modules");
        let send = || {
            post_with_key(
                &env,
                &owner.token,
                &path,
                "k-3",
                Some(json!({"title": "Together"})),
            )
        };
        let (a, b) = tokio::join!(send(), send());
        let mut statuses = [a.status, b.status];
        statuses.sort_unstable();
        assert!(
            statuses == [201, 201] || statuses == [201, 409],
            "one runs; the other replays it or is told to wait: {statuses:?}"
        );
        if statuses == [201, 409] {
            let waiting = if a.status == 409 { &a } else { &b };
            assert_eq!(waiting.header("retry-after"), "1");
        }
        assert_eq!(
            modules_titled(&env, &owner.token, course, "Together").await,
            1
        );
    })
    .await;
}

#[tokio::test]
async fn a_submit_that_is_sent_again_does_not_make_a_second_attempt() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let student = env.user("student@x.edu", Role::Student).await;
        let (_, module) = env.course(&owner, &[&student]).await;
        let assignment = env.assignment(&owner, module, "10").await;
        env.ok(
            "PUT",
            &student.token,
            &format!("/assignments/{assignment}/submission/draft"),
            Some(json!({"content": essay("x")})),
            200,
        )
        .await;

        let path = format!("/assignments/{assignment}/submission/submit");
        let first = post_with_key(&env, &student.token, &path, "submit-1", None).await;
        assert_eq!(first.status, 201);
        // The answer was lost on the way; the client asks again. Without the key this is a 409.
        let again = post_with_key(&env, &student.token, &path, "submit-1", None).await;
        assert_eq!(
            (again.status, again.body.clone()),
            (201, first.body.clone())
        );
        assert_eq!(again.header("idempotency-replayed"), "true");
        let plain = env.call("POST", &student.token, &path, None).await;
        assert_eq!(plain.status, 409, "no draft is left to submit");

        let mine = env
            .ok(
                "GET",
                &student.token,
                &format!("/assignments/{assignment}/submission"),
                None,
                200,
            )
            .await;
        assert_eq!(mine["attempts"].as_array().unwrap().len(), 1);
        let events = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM events.outbox WHERE type = 'submission.submitted'"#
        )
        .fetch_one(&env.pool)
        .await
        .unwrap();
        assert_eq!(events, 1);
    })
    .await;
}

#[tokio::test]
async fn an_answer_about_the_request_is_kept_even_when_it_was_a_refusal() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let student = env.user("student@x.edu", Role::Student).await;
        let (_, module) = env.course(&owner, &[&student]).await;
        let assignment = env.assignment(&owner, module, "10").await;
        let path = format!("/assignments/{assignment}/submission/submit");
        let refused = post_with_key(&env, &student.token, &path, "submit-2", None).await;
        assert_eq!(refused.status, 409, "nothing to submit yet");
        env.ok(
            "PUT",
            &student.token,
            &format!("/assignments/{assignment}/submission/draft"),
            Some(json!({"content": essay("x")})),
            200,
        )
        .await;
        // The same key is the same request: it gets the same answer. A new attempt takes a new key.
        let replay = post_with_key(&env, &student.token, &path, "submit-2", None).await;
        assert_eq!(
            (replay.status, replay.header("idempotency-replayed")),
            (409, "true")
        );
        let fresh = post_with_key(&env, &student.token, &path, "submit-3", None).await;
        assert_eq!(fresh.status, 201);
    })
    .await;
}

#[tokio::test]
async fn a_refused_credential_leaves_nothing_behind() {
    db_test(async |pool| {
        let (env, _owner, course) = teacher_world(pool).await;
        let response = post_with_key(
            &env,
            "not-a-token",
            &format!("/courses/{course}/modules"),
            "k-4",
            Some(json!({"title": "X"})),
        )
        .await;
        assert_eq!(response.status, 401);
        assert_eq!(
            stored_keys(&env.pool).await,
            0,
            "a 401 says nothing about the request"
        );
    })
    .await;
}

#[tokio::test]
async fn sign_in_is_not_covered_because_its_answer_is_a_credential() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        common::seed_user(&env.pool, "owner@x.edu", Role::Teacher).await;
        let login = || async {
            env.http
                .post("/auth/login")
                .header("idempotency-key", "login-1")
                .json(
                    &json!({"email": "owner@x.edu", "password": common::PASSWORD, "client": "app"}),
                )
                .send()
                .await
        };
        let (first, second) = (login().await, login().await);
        assert_eq!((first.status, second.status), (200, 200));
        assert_ne!(first.json()["token"], second.json()["token"]);
        assert_eq!(stored_keys(&env.pool).await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_key_that_cannot_be_one_is_refused_before_anything_runs() {
    db_test(async |pool| {
        let (env, owner, course) = teacher_world(pool).await;
        let path = format!("/courses/{course}/modules");
        for key in ["has space".to_owned(), "k".repeat(256)] {
            let response = post_with_key(
                &env,
                &owner.token,
                &path,
                &key,
                Some(json!({"title": "Never"})),
            )
            .await;
            assert_eq!(response.status, 422, "{key:?}");
        }
        assert_eq!(modules_titled(&env, &owner.token, course, "Never").await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_record_lasts_a_day_and_a_lost_request_is_taken_over() {
    db_test(async |pool| {
        let (env, owner, course) = teacher_world(pool).await;
        let path = format!("/courses/{course}/modules");
        let send = || {
            post_with_key(
                &env,
                &owner.token,
                &path,
                "k-5",
                Some(json!({"title": "Late"})),
            )
        };
        assert_eq!(send().await.status, 201);

        // Expired: the key is free, the request runs again.
        sqlx::query!("UPDATE idempotency.keys SET expires_at = now() - interval '1 second'")
            .execute(&env.pool)
            .await
            .unwrap();
        let again = send().await;
        assert_eq!(
            (again.status, again.headers.get("idempotency-replayed")),
            (201, None)
        );
        assert_eq!(modules_titled(&env, &owner.token, course, "Late").await, 2);

        // Still running (a request that has not answered yet): the retry is told to wait.
        sqlx::query!("UPDATE idempotency.keys SET status = NULL, content_type = NULL, body = NULL")
            .execute(&env.pool)
            .await
            .unwrap();
        let waiting = send().await;
        assert_eq!((waiting.status, waiting.header("retry-after")), (409, "1"));

        // Running for far longer than any request can: its process is gone, and the retry runs it.
        sqlx::query!("UPDATE idempotency.keys SET started_at = now() - interval '10 minutes'")
            .execute(&env.pool)
            .await
            .unwrap();
        assert_eq!(send().await.status, 201);
        assert_eq!(modules_titled(&env, &owner.token, course, "Late").await, 3);
    })
    .await;
}
