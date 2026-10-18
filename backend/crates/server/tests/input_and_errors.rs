//! Input the database cannot store, requests that lose a race, a database that is busy, and
//! bodies that cannot be read (ADR-0039): all of them end as a 4xx or a 503, never as a 500.

mod common;

use std::time::Duration;

use common::{Env, db_test, essay, id_of, wait_until_blocked};
use serde_json::{Value, json};
use server::identity::Role;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn paragraph_with(text: &str) -> Value {
    essay(text)
}

fn fields(response: &common::Resp) -> Vec<String> {
    response
        .problem(422)
        .errors
        .into_iter()
        .map(|error| error.field)
        .collect()
}

// --- control characters in documents (A-07) ---------------------------------------------------

#[tokio::test]
async fn a_control_character_in_a_document_is_a_422_on_its_path_and_a_line_break_is_fine() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let student = env.user("student@x.edu", Role::Student).await;
        let (_course, module) = env.course(&owner, &[&student]).await;
        let assignment = env.assignment(&owner, module, "10").await;
        let items = format!("/modules/{module}/items");
        let draft = format!("/assignments/{assignment}/submission/draft");

        // A student's draft, the case that used to answer 500 on every autosave.
        let nul = env
            .call("PUT", &student.token, &draft, Some(json!({"content": paragraph_with("hi\u{0}there")})))
            .await;
        assert_eq!(fields(&nul), ["content.blocks[0].content[0].text"]);
        // Nothing was saved.
        let mine = env
            .ok("GET", &student.token, &format!("/assignments/{assignment}/submission"), None, 200)
            .await;
        assert_eq!(mine["draft"], Value::Null);
        // Staff content: a text run, a code block, a formula and an inline formula.
        let bodies = [
            ("body.blocks[0].content[0].text", paragraph_with("a\u{1}b")),
            (
                "body.blocks[0].props.source",
                json!({"v": 1, "attachments": [], "blocks": [
                    {"id": "c", "type": "code", "props": {"language": "rust", "source": "x\u{0}y"}}]}),
            ),
            (
                "body.blocks[0].props.source",
                json!({"v": 1, "attachments": [], "blocks": [
                    {"id": "m", "type": "math", "props": {"source": "x\u{7}"}}]}),
            ),
            (
                "body.blocks[0].content[0].source",
                json!({"v": 1, "attachments": [], "blocks": [
                    {"id": "p", "type": "paragraph", "content": [{"type": "math", "source": "a\u{0}"}]}]}),
            ),
        ];
        for (field, body) in bodies {
            let r = env
                .call("POST", &owner.token, &items, Some(json!({"kind": "text", "title": "t", "body": body})))
                .await;
            assert_eq!(fields(&r), [field]);
        }
        // Line breaks and tabs are text.
        env.ok(
            "PUT",
            &student.token,
            &draft,
            Some(json!({"content": paragraph_with("line one\nline\ttwo\r\n")})),
            200,
        )
        .await;
        env.ok(
            "POST",
            &owner.token,
            &items,
            Some(json!({"kind": "text", "title": "t", "body": paragraph_with("a\nb")})),
            201,
        )
        .await;
    })
    .await;
}

// --- term dates the database cannot hold (A-07) -----------------------------------------------

#[tokio::test]
async fn a_term_date_outside_the_years_1_to_9999_is_a_422_on_its_field() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let term = |name: &str, from: &str, to: &str| {
            Some(json!({"name": name, "starts_on": from, "ends_on": to}))
        };
        let early = env
            .call(
                "POST",
                &admin.token,
                "/terms",
                term("early", "-5000-01-01", "2026-01-01"),
            )
            .await;
        assert_eq!(fields(&early), ["starts_on"]);
        let zero = env
            .call(
                "POST",
                &admin.token,
                "/terms",
                term("zero", "0000-01-01", "2026-01-01"),
            )
            .await;
        assert_eq!(fields(&zero), ["starts_on"]);
        let late = env
            .call(
                "POST",
                &admin.token,
                "/terms",
                term("late", "2026-01-01", "+10000-01-01"),
            )
            .await;
        assert_eq!(fields(&late), ["ends_on"]);
        let ok = env
            .ok(
                "POST",
                &admin.token,
                "/terms",
                term("ok", "0001-01-01", "9999-12-31"),
                201,
            )
            .await;
        // A patch is judged on the merged dates.
        let patched = env
            .call(
                "PATCH",
                &admin.token,
                &format!("/terms/{}", ok["id"].as_str().unwrap()),
                Some(json!({"ends_on": "-0100-01-01"})),
            )
            .await;
        assert_eq!(fields(&patched), ["ends_on"]);
    })
    .await;
}

// --- a write that loses a race to a delete (A-08) ---------------------------------------------

async fn audit_rows(pool: &sqlx::PgPool, action: &str) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE action = $1"#,
        action
    )
    .fetch_one(pool)
    .await
    .expect("audit rows")
}

/// A request that waits for a row another transaction has deleted, then finds it gone.
#[tokio::test]
async fn a_patch_that_waited_for_a_delete_is_a_404() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let (course, module) = env.course(&owner, &[]).await;

        // What each case patches: a module, a text item, and an assignment by its title and by
        // a field that takes the assignment row lock. Every case gets a row of its own.
        for case in [
            "module",
            "text item",
            "assignment title",
            "assignment points",
        ] {
            let (target, table, body) = match case {
                "module" => {
                    let path = format!("/courses/{course}/modules");
                    let created = env
                        .ok(
                            "POST",
                            &owner.token,
                            &path,
                            Some(json!({"title": "m"})),
                            201,
                        )
                        .await;
                    (id_of(&created), "modules", json!({"title": "renamed"}))
                }
                "text item" => {
                    let path = format!("/modules/{module}/items");
                    let body = json!({"kind": "text", "title": "t"});
                    let created = env.ok("POST", &owner.token, &path, Some(body), 201).await;
                    (id_of(&created), "items", json!({"title": "renamed"}))
                }
                "assignment title" => (
                    env.assignment(&owner, module, "10").await,
                    "items",
                    json!({"title": "renamed"}),
                ),
                _ => (
                    env.assignment(&owner, module, "10").await,
                    "items",
                    json!({"max_points": "5"}),
                ),
            };
            let path = if table == "modules" {
                format!("/modules/{target}")
            } else {
                format!("/items/{target}")
            };
            // The rows are deleted by hand in a transaction that stays open; the request waits.
            let mut tx = pool.begin().await.unwrap();
            let delete = if table == "modules" {
                "DELETE FROM content.modules WHERE id = $1"
            } else {
                "DELETE FROM content.items WHERE id = $1"
            };
            sqlx::query(delete)
                .bind(target)
                .execute(&mut *tx)
                .await
                .unwrap();
            let request = tokio::spawn({
                let (base, token) = (env.base.clone(), owner.token.clone());
                async move {
                    common::Http::new(&base)
                        .patch(&path)
                        .bearer(&token)
                        .json(&body)
                        .send()
                        .await
                }
            });
            wait_until_blocked(&pool, &request).await;
            tx.commit().await.unwrap();
            assert_eq!(request.await.unwrap().problem(404).status, 404, "{case}");
        }
    })
    .await;
}

/// Two deletes of one module: the one that lost answers 404 and writes no audit row.
#[tokio::test]
async fn a_delete_that_lost_the_race_to_another_delete_is_a_404_without_an_audit_row() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let (course, module) = env.course(&owner, &[]).await;
        let item = id_of(
            &env.ok(
                "POST",
                &owner.token,
                &format!("/modules/{module}/items"),
                Some(json!({"kind": "text", "title": "t"})),
                201,
            )
            .await,
        );
        let doomed = id_of(
            &env.ok(
                "POST",
                &owner.token,
                &format!("/courses/{course}/modules"),
                Some(json!({"title": "doomed"})),
                201,
            )
            .await,
        );
        for (path, delete, action) in [
            (
                format!("/modules/{doomed}"),
                ("DELETE FROM content.modules WHERE id = $1", doomed),
                "content.module.delete",
            ),
            (
                format!("/items/{item}"),
                ("DELETE FROM content.items WHERE id = $1", item),
                "content.item.delete",
            ),
        ] {
            let (sql, id) = delete;
            let before = audit_rows(&pool, action).await;
            let mut tx = pool.begin().await.unwrap();
            sqlx::query(sql).bind(id).execute(&mut *tx).await.unwrap();
            // An admin acts as the owner, so a delete that really happens is audited.
            let request = tokio::spawn({
                let (base, token) = (env.base.clone(), admin.token.clone());
                async move {
                    common::Http::new(&base)
                        .delete(&path)
                        .bearer(&token)
                        .send()
                        .await
                }
            });
            wait_until_blocked(&pool, &request).await;
            tx.commit().await.unwrap();
            assert_eq!(request.await.unwrap().problem(404).status, 404, "{action}");
            assert_eq!(
                audit_rows(&pool, action).await,
                before,
                "{action}: no row for what did not happen"
            );
        }
        // A delete that wins is audited once.
        let winner = id_of(
            &env.ok(
                "POST",
                &owner.token,
                &format!("/courses/{course}/modules"),
                Some(json!({"title": "w"})),
                201,
            )
            .await,
        );
        env.ok(
            "DELETE",
            &admin.token,
            &format!("/modules/{winner}"),
            None,
            204,
        )
        .await;
        assert_eq!(audit_rows(&pool, "content.module.delete").await, 1);
    })
    .await;
}

/// Publishing something that was deleted while the request waited is a 404, not a stale 200.
#[tokio::test]
async fn publishing_a_row_that_was_deleted_meanwhile_is_a_404() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let (course, _module) = env.course(&owner, &[]).await;
        let doomed = id_of(
            &env.ok(
                "POST",
                &owner.token,
                &format!("/courses/{course}/modules"),
                Some(json!({"title": "d"})),
                201,
            )
            .await,
        );
        // Published already: the update matches no row to change, and the module is still there.
        env.ok(
            "POST",
            &owner.token,
            &format!("/modules/{doomed}/publish"),
            Some(json!({"cascade": false})),
            200,
        )
        .await;
        env.ok(
            "POST",
            &owner.token,
            &format!("/modules/{doomed}/publish"),
            Some(json!({"cascade": false})),
            200,
        )
        .await;
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM content.modules WHERE id = $1")
            .bind(doomed)
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let gone = env
            .call(
                "POST",
                &owner.token,
                &format!("/modules/{doomed}/publish"),
                Some(json!({"cascade": false})),
            )
            .await;
        assert_eq!(gone.status, 404);
    })
    .await;
}

// --- a busy database is a 503 (A-11) ----------------------------------------------------------

#[tokio::test]
async fn a_pool_with_no_connection_to_give_is_a_503_with_retry_after() {
    db_test(async |pool| {
        // The server gets a pool of one connection and a short patience; the test holds it.
        let small = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_millis(500))
            .connect_with((*pool.connect_options()).clone())
            .await
            .unwrap();
        let base = common::spawn(small.clone()).await;
        let http = common::Http::new(&base);
        let (_id, token) = common::seed_and_login(&pool, &http, "u@x.edu", Role::Teacher).await;
        let held = small.acquire().await.unwrap();

        for path in ["/me/courses", "/me", "/health"] {
            let r = http.get(path).bearer(&token).send().await;
            let problem = r.problem(503);
            assert_eq!(problem.status, 503, "{path}");
            assert!(r.headers.contains_key("retry-after"), "{path}");
        }
        drop(held);
        assert_eq!(
            http.get("/me/courses").bearer(&token).send().await.status,
            200
        );
    })
    .await;
}

// --- bodies that cannot be read (A-14) --------------------------------------------------------

#[tokio::test]
async fn a_body_that_cannot_be_read_is_a_422_and_a_wrong_media_type_a_415() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let client = reqwest::Client::new();
        let url = format!("{}/api/v1/courses", env.base);
        let post = |body: &str, content_type: Option<&str>| {
            let mut request = client
                .post(&url)
                .bearer_auth(&owner.token)
                .body(body.to_owned());
            if let Some(content_type) = content_type {
                request = request.header("content-type", content_type);
            }
            request
        };
        let status = async |request: reqwest::RequestBuilder| {
            let response = request.send().await.unwrap();
            let status = response.status().as_u16();
            let content_type = response.headers()["content-type"]
                .to_str()
                .unwrap()
                .to_owned();
            (status, content_type)
        };
        let problem = "application/problem+json";
        assert_eq!(
            status(post("{\"title\": ", Some("application/json"))).await,
            (422, problem.into())
        );
        assert_eq!(
            status(post("title=x", Some("application/json"))).await,
            (422, problem.into())
        );
        assert_eq!(
            status(post("{\"title\": 5}", Some("application/json"))).await,
            (422, problem.into())
        );
        assert_eq!(
            status(post("{\"title\": \"x\"}", None)).await,
            (415, problem.into())
        );
        assert_eq!(
            status(post(
                "{\"title\": \"x\"}",
                Some("application/x-www-form-urlencoded")
            ))
            .await,
            (415, problem.into())
        );
        let deep = format!("{}1{}", "[".repeat(300), "]".repeat(300));
        assert_eq!(
            status(post(&deep, Some("application/json"))).await,
            (422, problem.into())
        );
        // No echo of what was sent.
        let secret = post("{\"title\": \"leaky-value\" nope", Some("application/json"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!secret.contains("leaky-value"), "{secret}");
    })
    .await;
}

// --- an attempts cursor belongs to its student and assignment (cursor scoping) ---------------

#[tokio::test]
async fn a_cursor_from_another_students_attempts_gives_nothing() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let [a, b] = [
            env.user("a@x.edu", Role::Student).await,
            env.user("b@x.edu", Role::Student).await,
        ];
        let (_course, module) = env.course(&owner, &[&a, &b]).await;
        let assignment = env.assignment(&owner, module, "10").await;
        let own = format!("/assignments/{assignment}/submission");
        let turn_in = async |who: &common::Account, text: &str| {
            env.ok(
                "PUT",
                &who.token,
                &format!("{own}/draft"),
                Some(json!({"content": essay(text)})),
                200,
            )
            .await;
            id_of(
                &env.ok("POST", &who.token, &format!("{own}/submit"), None, 201)
                    .await,
            )
        };
        let [_a1, a2] = [turn_in(&a, "one").await, turn_in(&a, "two").await];
        let [_b1, b2] = [turn_in(&b, "uno").await, turn_in(&b, "dos").await];
        let attempts = async |who: &common::Account, cursor: Uuid| {
            let page = env
                .ok(
                    "GET",
                    &who.token,
                    &format!("{own}?cursor={cursor}"),
                    None,
                    200,
                )
                .await;
            page["attempts"].as_array().unwrap().len()
        };
        // Their own cursor works: the attempt before attempt 2 is attempt 1.
        assert_eq!(attempts(&a, a2).await, 1);
        // Another student's attempt id says nothing about this student's attempts.
        assert_eq!(attempts(&a, b2).await, 0);
        // Nor does the id of a row of another assignment.
        let other = env.assignment(&owner, module, "10").await;
        env.ok(
            "PUT",
            &a.token,
            &format!("/assignments/{other}/submission/draft"),
            Some(json!({"content": essay("x")})),
            200,
        )
        .await;
        let elsewhere = id_of(
            &env.ok(
                "POST",
                &a.token,
                &format!("/assignments/{other}/submission/submit"),
                None,
                201,
            )
            .await,
        );
        assert_eq!(attempts(&a, elsewhere).await, 0);
    })
    .await;
}
