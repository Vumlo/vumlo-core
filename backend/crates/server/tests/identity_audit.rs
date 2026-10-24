//! The audit of admin writes in the identity admin API (ADR-0034).

mod common;

use ::common::Role;
use common::{Http, Resp, db_test, seed_and_login};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

struct World {
    http: Http,
    pool: PgPool,
    admin: (Uuid, String),
    teacher: (Uuid, String),
    student: (Uuid, String),
}

/// An admin, a teacher and a student, seeded the way tests seed: the log starts empty.
async fn world(pool: PgPool) -> World {
    let http = Http::new(&common::spawn(pool.clone()).await);
    let admin = seed_and_login(&pool, &http, "admin@x.edu", Role::Admin).await;
    let teacher = seed_and_login(&pool, &http, "teacher@x.edu", Role::Teacher).await;
    let student = seed_and_login(&pool, &http, "student@x.edu", Role::Student).await;
    let w = World {
        http,
        pool,
        admin,
        teacher,
        student,
    };
    assert!(w.rows().await.is_empty(), "seeding writes no audit row");
    w
}

struct Row {
    action: String,
    actor_id: Option<Uuid>,
    source: String,
    resource_type: String,
    resource_id: Uuid,
    meta: Value,
}

impl World {
    async fn rows(&self) -> Vec<Row> {
        sqlx::query!(
            "SELECT action, actor_id, source, resource_type, resource_id, meta
             FROM events.audit_log ORDER BY at, id"
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| Row {
            action: r.action,
            actor_id: r.actor_id,
            source: r.source,
            resource_type: r.resource_type,
            resource_id: r.resource_id,
            meta: r.meta,
        })
        .collect()
    }

    async fn send(&self, method: &str, token: &str, path: &str, body: Option<Value>) -> Resp {
        let request = match method {
            "GET" => self.http.get(path),
            "POST" => self.http.post(path),
            "PATCH" => self.http.patch(path),
            other => panic!("{other}"),
        }
        .bearer(token);
        match body {
            Some(body) => request.json(&body).send().await,
            None => request.send().await,
        }
    }

    async fn as_admin(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        self.send(method, &self.admin.1, path, body).await
    }

    async fn patch(&self, user: Uuid, body: Value) -> Resp {
        self.as_admin("PATCH", &format!("/admin/users/{user}"), Some(body))
            .await
    }
}

/// An audit row about `resource`, written by the admin through the API.
fn assert_row(row: &Row, w: &World, action: &str, resource: Uuid, meta: Value) {
    assert_eq!(row.action, action);
    assert_eq!(row.actor_id, Some(w.admin.0));
    assert_eq!(row.source, "api");
    assert_eq!(row.resource_type, "user");
    assert_eq!(row.resource_id, resource);
    assert_eq!(row.meta, meta);
}

/// No secret and no personal data in the meta of any row.
fn assert_clean(rows: &[Row], forbidden: &[&str]) {
    for row in rows {
        let meta = row.meta.to_string();
        for needle in forbidden
            .iter()
            .chain(&["argon2", "@x.edu", "password_hash", "token"])
        {
            assert!(!meta.contains(needle), "{needle} in {meta}");
        }
    }
}

#[tokio::test]
async fn creating_a_user_is_audited_without_the_password_or_the_email() {
    db_test(async |pool| {
        let w = world(pool).await;

        let created = w
            .as_admin(
                "POST",
                "/admin/users",
                Some(
                    json!({"email": "new@x.edu", "name": "Ada", "surname": "Lovelace",
                            "role": "teacher"}),
                ),
            )
            .await;
        assert_eq!(created.status, 201);
        let body = created.json();
        let id: Uuid = body["user"]["id"].as_str().unwrap().parse().unwrap();
        let temporary = body["temporary_password"].as_str().unwrap().to_owned();

        let rows = w.rows().await;
        assert_eq!(rows.len(), 1);
        assert_row(&rows[0], &w, "user.create", id, json!({"role": "teacher"}));
        assert_clean(&rows, &[&temporary, "Ada", "Lovelace"]);
    })
    .await;
}

#[tokio::test]
async fn renaming_changing_the_role_and_disabling_are_audited_one_row_each() {
    db_test(async |pool| {
        let w = world(pool).await;
        let target = w.student.0;

        // One request, three kinds of change: three rows, in a fixed order.
        let response = w
            .patch(
                target,
                json!({"name": "Grace", "surname": "Hopper", "role": "teacher", "status": "disabled"}),
            )
            .await;
        assert_eq!(response.status, 200);
        let rows = w.rows().await;
        assert_eq!(rows.len(), 3);
        assert_row(
            &rows[0],
            &w,
            "user.rename",
            target,
            json!({"fields": ["name", "surname"]}),
        );
        assert_row(
            &rows[1],
            &w,
            "user.role_change",
            target,
            json!({"from": "student", "to": "teacher"}),
        );
        assert_row(&rows[2], &w, "user.disable", target, json!({}));

        assert_eq!(
            w.patch(target, json!({"status": "active"})).await.status,
            200
        );
        assert_eq!(
            w.patch(target, json!({"name": "Grace M."})).await.status,
            200
        );
        assert_eq!(w.patch(target, json!({"role": "admin"})).await.status, 200);
        let rows = w.rows().await;
        assert_eq!(rows.len(), 6);
        assert_row(&rows[3], &w, "user.enable", target, json!({}));
        assert_row(
            &rows[4],
            &w,
            "user.rename",
            target,
            json!({"fields": ["name"]}),
        );
        assert_row(
            &rows[5],
            &w,
            "user.role_change",
            target,
            json!({"from": "teacher", "to": "admin"}),
        );
        assert_clean(&rows, &["Grace", "Hopper"]);
    })
    .await;
}

#[tokio::test]
async fn resetting_a_password_is_audited_without_the_new_password() {
    db_test(async |pool| {
        let w = world(pool).await;

        let response = w
            .as_admin(
                "POST",
                &format!("/admin/users/{}/reset-password", w.student.0),
                None,
            )
            .await;
        assert_eq!(response.status, 200);
        let temporary = response.json()["temporary_password"]
            .as_str()
            .unwrap()
            .to_owned();

        let rows = w.rows().await;
        assert_eq!(rows.len(), 1);
        assert_row(&rows[0], &w, "user.password_reset", w.student.0, json!({}));
        assert_clean(&rows, &[&temporary]);
    })
    .await;
}

#[tokio::test]
async fn a_change_that_changes_nothing_is_not_audited() {
    db_test(async |pool| {
        let w = world(pool).await;
        let name: String =
            sqlx::query_scalar!("SELECT name FROM auth.users WHERE id = $1", w.student.0)
                .fetch_one(&w.pool)
                .await
                .unwrap();

        let same = w
            .patch(
                w.student.0,
                json!({"name": name, "role": "student", "status": "active"}),
            )
            .await;
        assert_eq!(same.status, 200);
        assert_eq!(w.patch(w.student.0, json!({})).await.status, 200);

        assert!(w.rows().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_refused_or_failed_write_is_not_audited() {
    db_test(async |pool| {
        let w = world(pool).await;
        let unknown = Uuid::now_v7();

        // The last active admin cannot be demoted or disabled.
        w.patch(w.admin.0, json!({"role": "teacher"}))
            .await
            .problem(409);
        w.patch(w.admin.0, json!({"status": "disabled"}))
            .await
            .problem(409);
        // Even together with a change that alone would be fine: nothing of it stays.
        w.patch(w.admin.0, json!({"name": "Changed", "role": "teacher"}))
            .await
            .problem(409);
        // An email in use, bad fields, an unknown user.
        let duplicate =
            json!({"email": "teacher@x.edu", "name": "A", "surname": "B", "role": "student"});
        w.as_admin("POST", "/admin/users", Some(duplicate))
            .await
            .problem(409);
        let bad = json!({"email": "not-an-email", "name": "A", "surname": "B", "role": "student"});
        w.as_admin("POST", "/admin/users", Some(bad))
            .await
            .problem(422);
        w.patch(w.student.0, json!({"name": ""})).await.problem(422);
        w.patch(unknown, json!({"name": "X"})).await.problem(404);
        w.as_admin(
            "POST",
            &format!("/admin/users/{unknown}/reset-password"),
            None,
        )
        .await
        .problem(404);

        assert!(w.rows().await.is_empty());
        let name: String =
            sqlx::query_scalar!("SELECT name FROM auth.users WHERE id = $1", w.admin.0)
                .fetch_one(&w.pool)
                .await
                .unwrap();
        assert_ne!(name, "Changed", "the refused request changed nothing");
    })
    .await;
}

#[tokio::test]
async fn reads_are_not_audited() {
    db_test(async |pool| {
        let w = world(pool).await;

        assert_eq!(w.as_admin("GET", "/admin/users", None).await.status, 200);
        assert_eq!(w.as_admin("GET", "/me", None).await.status, 200);

        assert!(w.rows().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_non_admin_is_refused_and_nothing_is_audited() {
    db_test(async |pool| {
        let w = world(pool).await;
        let target = w.student.0;
        let create = json!({"email": "new@x.edu", "name": "A", "surname": "B", "role": "student"});
        let calls: Vec<(&str, String, Option<Value>)> = vec![
            ("POST", "/admin/users".into(), Some(create)),
            (
                "PATCH",
                format!("/admin/users/{target}"),
                Some(json!({"role": "admin"})),
            ),
            (
                "POST",
                format!("/admin/users/{target}/reset-password"),
                None,
            ),
        ];
        for (method, path, body) in &calls {
            for token in [&w.teacher.1, &w.student.1] {
                w.send(method, token, path, body.clone()).await.problem(403);
            }
            let anonymous = match method {
                &"POST" => w.http.post(path).send().await,
                _ => w.http.patch(path).send().await,
            };
            anonymous.problem(401);
        }

        assert!(w.rows().await.is_empty());
        let role = sqlx::query_scalar!("SELECT role FROM auth.users WHERE id = $1", target)
            .fetch_one(&w.pool)
            .await
            .unwrap();
        assert_eq!(role, "student");
    })
    .await;
}
