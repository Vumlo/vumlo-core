//! The installation record (ADR-0031): the first start records the mode, later starts must agree.
//! Terms, groups and subgroups over HTTP (ADR-0032) follow at the end.

mod common;

use std::time::Duration;

use ::common::Role;
use common::{Http, Resp, db_test, seed_and_login};
use serde_json::{Value, json};
use server::org::{self, OrgError};
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::InstitutionMode::{self, School, University};

async fn start(pool: &PgPool, mode: InstitutionMode) -> Result<(), OrgError> {
    let mut tx = pool.begin().await.unwrap();
    org::record_mode(&mut tx, mode).await?;
    tx.commit().await.unwrap();
    Ok(())
}

async fn recorded(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar!("SELECT mode FROM org.installation")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// Waits until some other session of this database is blocked on a lock. That is the second
/// starter waiting for the first one's uncommitted row, so the race is really exercised and
/// the steps do not depend on timing.
async fn wait_for_a_blocked_session(pool: &PgPool) {
    for _ in 0..200 {
        let blocked = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM pg_stat_activity
               WHERE datname = current_database() AND wait_event_type = 'Lock'"#
        )
        .fetch_one(pool)
        .await
        .unwrap();
        if blocked > 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the second starter never blocked on the first");
}

#[tokio::test]
async fn the_first_start_records_the_mode() {
    db_test(async |pool| {
        assert!(recorded(&pool).await.is_empty());

        start(&pool, School).await.unwrap();

        assert_eq!(recorded(&pool).await, ["school"]);
    })
    .await;
}

#[tokio::test]
async fn a_restart_with_the_same_mode_passes_and_changes_nothing() {
    db_test(async |pool| {
        start(&pool, University).await.unwrap();
        start(&pool, University).await.unwrap();

        assert_eq!(recorded(&pool).await, ["university"]);
    })
    .await;
}

#[tokio::test]
async fn a_different_mode_is_refused_and_the_message_names_both_values() {
    db_test(async |pool| {
        start(&pool, School).await.unwrap();

        let error = start(&pool, University).await.unwrap_err();

        assert!(
            matches!(
                error,
                OrgError::ModeChanged {
                    recorded: School,
                    configured: University
                }
            ),
            "{error}"
        );
        let message = error.to_string();
        for needle in ["institution.mode", "school", "university"] {
            assert!(message.contains(needle), "{needle}: {message}");
        }
        assert_eq!(recorded(&pool).await, ["school"]);
    })
    .await;
}

/// A records and has not committed; B starts meanwhile, has to wait for A's row, and is let go
/// when A commits. Returns B's outcome; A must always pass.
async fn second_starter_passes(
    pool: &PgPool,
    first: InstitutionMode,
    second: InstitutionMode,
) -> bool {
    let mut a = pool.begin().await.unwrap();
    org::record_mode(&mut a, first).await.unwrap();

    let b = tokio::spawn({
        let pool = pool.clone();
        async move { start(&pool, second).await.is_ok() }
    });
    wait_for_a_blocked_session(pool).await;
    a.commit().await.unwrap();

    b.await.unwrap()
}

#[tokio::test]
async fn two_first_starts_with_the_same_mode_both_pass() {
    db_test(async |pool| {
        assert!(second_starter_passes(&pool, School, School).await);
        assert_eq!(recorded(&pool).await, ["school"]);
    })
    .await;
}

#[tokio::test]
async fn two_first_starts_with_different_modes_exactly_one_passes() {
    db_test(async |pool| {
        assert!(!second_starter_passes(&pool, School, University).await);
        assert_eq!(recorded(&pool).await, ["school"]);
    })
    .await;
}

#[tokio::test]
async fn a_starter_that_rolls_back_does_not_block_the_next_one() {
    db_test(async |pool| {
        let mut a = pool.begin().await.unwrap();
        org::record_mode(&mut a, School).await.unwrap();
        a.rollback().await.unwrap();

        start(&pool, University).await.unwrap();

        assert_eq!(recorded(&pool).await, ["university"]);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Terms, groups and subgroups (ADR-0032)
// ---------------------------------------------------------------------------------------------

struct User {
    id: Uuid,
    token: String,
}

struct World {
    http: Http,
    pool: PgPool,
    admin: User,
    teacher: User,
    student: User,
}

async fn user(pool: &PgPool, http: &Http, email: &str, role: Role) -> User {
    let (id, token) = seed_and_login(pool, http, email, role).await;
    User { id, token }
}

async fn world(pool: PgPool) -> World {
    let http = Http::new(&common::spawn(pool.clone()).await);
    let admin = user(&pool, &http, "admin@x.edu", Role::Admin).await;
    let teacher = user(&pool, &http, "teacher@x.edu", Role::Teacher).await;
    let student = user(&pool, &http, "student@x.edu", Role::Student).await;
    World {
        http,
        pool,
        admin,
        teacher,
        student,
    }
}

/// One request by method name, with or without a token.
async fn call(
    http: &Http,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Resp {
    let request = match method {
        "GET" => http.get(path),
        "POST" => http.post(path),
        "PATCH" => http.patch(path),
        "DELETE" => http.delete(path),
        other => panic!("method {other}"),
    };
    let request = match token {
        Some(token) => request.bearer(token),
        None => request,
    };
    let request = match body {
        Some(body) => request.json(body),
        None => request,
    };
    request.send().await
}

impl World {
    async fn as_admin(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        call(
            &self.http,
            method,
            path,
            Some(&self.admin.token),
            body.as_ref(),
        )
        .await
    }

    async fn term(&self, name: &str) -> Uuid {
        let response = self
            .as_admin(
                "POST",
                "/terms",
                Some(json!({"name": name, "starts_on": "2026-09-01", "ends_on": "2027-01-31"})),
            )
            .await;
        assert_eq!(
            response.status,
            201,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        common::id_of(&response.json())
    }

    async fn group(&self, name: &str) -> Uuid {
        let response = self
            .as_admin("POST", "/groups", Some(json!({"name": name})))
            .await;
        assert_eq!(
            response.status,
            201,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        common::id_of(&response.json())
    }

    async fn subgroup(&self, group: Uuid, name: &str) -> Uuid {
        let response = self
            .as_admin(
                "POST",
                &format!("/groups/{group}/subgroups"),
                Some(json!({"name": name})),
            )
            .await;
        assert_eq!(
            response.status,
            201,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        common::id_of(&response.json())
    }

    async fn new_student(&self, email: &str) -> Uuid {
        user(&self.pool, &self.http, email, Role::Student).await.id
    }

    async fn add_to_group(&self, group: Uuid, users: &[Uuid]) -> Resp {
        self.as_admin(
            "POST",
            &format!("/groups/{group}/members"),
            Some(json!({"user_ids": users})),
        )
        .await
    }

    async fn add_to_subgroup(&self, subgroup: Uuid, users: &[Uuid]) -> Resp {
        self.as_admin(
            "POST",
            &format!("/subgroups/{subgroup}/members"),
            Some(json!({"user_ids": users})),
        )
        .await
    }

    async fn group_member_count(&self, group: Uuid) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM org.group_members WHERE group_id = $1"#,
            group
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn subgroup_member_count(&self, subgroup: Uuid) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM org.subgroup_members WHERE subgroup_id = $1"#,
            subgroup
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn patch_user(&self, id: Uuid, body: Value) {
        let response = self
            .as_admin("PATCH", &format!("/admin/users/{id}"), Some(body))
            .await;
        assert_eq!(response.status, 200);
    }

    async fn set_names(&self, user: Uuid, name: &str, surname: &str) {
        sqlx::query!(
            "UPDATE auth.users SET name = $2, surname = $3 WHERE id = $1",
            user,
            name,
            surname
        )
        .execute(&self.pool)
        .await
        .unwrap();
    }
}

fn ids(page: &Value, key: &str) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item[key].as_str().unwrap().to_owned())
        .collect()
}

fn fields(response: &Resp) -> Vec<String> {
    response
        .problem(422)
        .errors
        .into_iter()
        .map(|error| error.field)
        .collect()
}

#[tokio::test]
async fn every_endpoint_answers_an_admin_a_teacher_a_student_and_an_anonymous_caller() {
    db_test(async |pool| {
        let w = world(pool).await;
        let term = w.term("Fall 2026").await;
        let group = w.group("7-A").await;
        let subgroup = w.subgroup(group, "English 1").await;
        let member = w.student.id;
        assert_eq!(w.add_to_group(group, &[member]).await.status, 200);
        let newcomer = w.new_student("newcomer@x.edu").await;

        // (method, path, body, the admin's status, may a teacher and a student read it)
        let cases: Vec<(&str, String, Option<Value>, u16, bool)> = vec![
            (
                "POST",
                "/terms".into(),
                Some(
                    json!({"name": "Spring 2027", "starts_on": "2027-02-01", "ends_on": "2027-06-30"}),
                ),
                201,
                false,
            ),
            ("GET", "/terms".into(), None, 200, true),
            ("GET", format!("/terms/{term}"), None, 200, true),
            (
                "PATCH",
                format!("/terms/{term}"),
                Some(json!({"name": "Autumn 2026"})),
                200,
                false,
            ),
            (
                "POST",
                "/groups".into(),
                Some(json!({"name": "7-B"})),
                201,
                false,
            ),
            ("GET", "/groups".into(), None, 200, false),
            ("GET", format!("/groups/{group}"), None, 200, false),
            (
                "PATCH",
                format!("/groups/{group}"),
                Some(json!({"name": "7-A (2026)"})),
                200,
                false,
            ),
            ("GET", format!("/groups/{group}/members"), None, 200, false),
            (
                "POST",
                format!("/groups/{group}/members"),
                Some(json!({"user_ids": [newcomer]})),
                200,
                false,
            ),
            (
                "POST",
                format!("/groups/{group}/subgroups"),
                Some(json!({"name": "English 2"})),
                201,
                false,
            ),
            (
                "GET",
                format!("/groups/{group}/subgroups"),
                None,
                200,
                false,
            ),
            (
                "PATCH",
                format!("/subgroups/{subgroup}"),
                Some(json!({"name": "English one"})),
                200,
                false,
            ),
            (
                "GET",
                format!("/subgroups/{subgroup}/members"),
                None,
                200,
                false,
            ),
            (
                "POST",
                format!("/subgroups/{subgroup}/members"),
                Some(json!({"user_ids": [member]})),
                200,
                false,
            ),
            (
                "DELETE",
                format!("/subgroups/{subgroup}/members/{member}"),
                None,
                204,
                false,
            ),
            ("DELETE", format!("/subgroups/{subgroup}"), None, 204, false),
            (
                "DELETE",
                format!("/groups/{group}/members/{newcomer}"),
                None,
                204,
                false,
            ),
            ("DELETE", format!("/groups/{group}"), None, 204, false),
            ("DELETE", format!("/terms/{term}"), None, 204, false),
        ];

        // Everyone else first, so the admin's deletes do not take the resources away.
        for (method, path, body, _, open) in &cases {
            let anonymous = call(&w.http, method, path, None, body.as_ref()).await;
            assert_eq!(anonymous.status, 401, "anonymous {method} {path}");
            for who in [&w.teacher, &w.student] {
                let response = call(&w.http, method, path, Some(&who.token), body.as_ref()).await;
                if *open {
                    assert_eq!(response.status, 200, "{method} {path}");
                } else {
                    response.problem(403);
                }
            }
        }
        for (method, path, body, status, _) in &cases {
            let response = call(&w.http, method, path, Some(&w.admin.token), body.as_ref()).await;
            assert_eq!(
                response.status,
                *status,
                "admin {method} {path}: {}",
                String::from_utf8_lossy(&response.body)
            );
        }
    })
    .await;
}

#[tokio::test]
async fn an_admin_creates_reads_updates_and_deletes_a_term() {
    db_test(async |pool| {
        let w = world(pool).await;

        let created = w
            .as_admin(
                "POST",
                "/terms",
                Some(
                    json!({"name": "  Fall 2026 ", "starts_on": "2026-09-01", "ends_on": "2027-01-31"}),
                ),
            )
            .await;
        assert_eq!(created.status, 201);
        let term = created.json();
        assert_eq!(term["name"], "Fall 2026");
        assert_eq!(term["starts_on"], "2026-09-01");
        assert_eq!(term["ends_on"], "2027-01-31");
        let id = common::id_of(&term);

        let read = call(
            &w.http,
            "GET",
            &format!("/terms/{id}"),
            Some(&w.student.token),
            None,
        )
        .await;
        assert_eq!(read.json(), term);

        let updated = w
            .as_admin(
                "PATCH",
                &format!("/terms/{id}"),
                Some(json!({"ends_on": "2027-02-15"})),
            )
            .await;
        assert_eq!(updated.status, 200);
        assert_eq!(updated.json()["ends_on"], "2027-02-15");
        assert_eq!(updated.json()["name"], "Fall 2026");

        assert_eq!(
            w.as_admin("DELETE", &format!("/terms/{id}"), None)
                .await
                .status,
            204
        );
        w.as_admin("GET", &format!("/terms/{id}"), None)
            .await
            .problem(404);
        w.as_admin("DELETE", &format!("/terms/{id}"), None)
            .await
            .problem(404);
    })
    .await;
}

#[tokio::test]
async fn a_term_needs_a_name_that_is_free_and_dates_in_order() {
    db_test(async |pool| {
        let w = world(pool).await;
        let first = w.term("Fall 2026").await;
        let other = w.term("Spring 2027").await;
        let body = |name: &str, starts: &str, ends: &str| json!({"name": name, "starts_on": starts, "ends_on": ends});

        let duplicate = w
            .as_admin(
                "POST",
                "/terms",
                Some(body(" Fall 2026", "2026-09-01", "2027-01-31")),
            )
            .await;
        assert_eq!(
            duplicate.problem(409).detail.as_deref(),
            Some("a term with this name already exists")
        );
        w.as_admin(
            "PATCH",
            &format!("/terms/{other}"),
            Some(json!({"name": "Fall 2026"})),
        )
        .await
        .problem(409);

        for name in ["", "   ", "a\nb"] {
            let response = w
                .as_admin(
                    "POST",
                    "/terms",
                    Some(body(name, "2026-09-01", "2027-01-31")),
                )
                .await;
            assert_eq!(fields(&response), ["name"], "{name:?}");
        }
        let too_long = "x".repeat(101);
        let response = w
            .as_admin(
                "POST",
                "/terms",
                Some(body(&too_long, "2026-09-01", "2027-01-31")),
            )
            .await;
        assert_eq!(fields(&response), ["name"]);

        let backwards = w
            .as_admin(
                "POST",
                "/terms",
                Some(body("Backwards", "2027-01-31", "2026-09-01")),
            )
            .await;
        assert_eq!(fields(&backwards), ["ends_on"]);
        let one_day = w
            .as_admin(
                "POST",
                "/terms",
                Some(body("One day", "2026-09-01", "2026-09-01")),
            )
            .await;
        assert_eq!(one_day.status, 201);

        // The merged dates are checked, not only the ones that were sent.
        let moved = w
            .as_admin(
                "PATCH",
                &format!("/terms/{first}"),
                Some(json!({"starts_on": "2027-02-01"})),
            )
            .await;
        assert_eq!(fields(&moved), ["ends_on"]);
        let stored = w
            .as_admin("GET", &format!("/terms/{first}"), None)
            .await
            .json();
        assert_eq!(stored["starts_on"], "2026-09-01");
        w.as_admin(
            "PATCH",
            &format!("/terms/{}", Uuid::now_v7()),
            Some(json!({"name": "x"})),
        )
        .await
        .problem(404);
    })
    .await;
}

#[tokio::test]
async fn terms_and_groups_are_listed_in_pages() {
    db_test(async |pool| {
        let w = world(pool).await;
        let mut terms = Vec::new();
        let mut groups = Vec::new();
        for n in 0..5 {
            terms.push(w.term(&format!("Term {n}")).await.to_string());
            groups.push(w.group(&format!("Group {n}")).await.to_string());
        }

        for (path, expected, token) in [
            ("/terms", &terms, &w.student.token),
            ("/groups", &groups, &w.admin.token),
        ] {
            let first = call(
                &w.http,
                "GET",
                &format!("{path}?limit=2"),
                Some(token),
                None,
            )
            .await
            .json();
            assert_eq!(ids(&first, "id"), expected[..2]);
            let cursor = first["next_cursor"].as_str().unwrap();
            let second = call(
                &w.http,
                "GET",
                &format!("{path}?limit=2&cursor={cursor}"),
                Some(token),
                None,
            )
            .await
            .json();
            assert_eq!(ids(&second, "id"), expected[2..4]);
            let cursor = second["next_cursor"].as_str().unwrap();
            let last = call(
                &w.http,
                "GET",
                &format!("{path}?limit=2&cursor={cursor}"),
                Some(token),
                None,
            )
            .await
            .json();
            assert_eq!(ids(&last, "id"), expected[4..]);
            assert_eq!(last["next_cursor"], Value::Null);
            call(
                &w.http,
                "GET",
                &format!("{path}?cursor=junk"),
                Some(token),
                None,
            )
            .await
            .problem(422);
        }
    })
    .await;
}

#[tokio::test]
async fn group_and_subgroup_names_are_trimmed_and_unique() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("  7-A ").await;
        let other = w.group("7-B").await;
        let shown = w
            .as_admin("GET", &format!("/groups/{group}"), None)
            .await
            .json();
        assert_eq!(shown["name"], "7-A");

        w.as_admin("POST", "/groups", Some(json!({"name": "7-A"})))
            .await
            .problem(409);
        w.as_admin(
            "PATCH",
            &format!("/groups/{other}"),
            Some(json!({"name": "7-A"})),
        )
        .await
        .problem(409);
        assert_eq!(
            w.as_admin(
                "PATCH",
                &format!("/groups/{group}"),
                Some(json!({"name": "7-A"}))
            )
            .await
            .status,
            200,
            "renaming a group to its own name is allowed"
        );
        for name in ["", "  ", "a\tb"] {
            let response = w
                .as_admin("POST", "/groups", Some(json!({"name": name})))
                .await;
            assert_eq!(fields(&response), ["name"], "{name:?}");
        }

        let sub = w.subgroup(group, "English 1").await;
        w.as_admin(
            "POST",
            &format!("/groups/{group}/subgroups"),
            Some(json!({"name": "English 1"})),
        )
        .await
        .problem(409);
        // The same name in another group is a different subgroup.
        w.subgroup(other, "English 1").await;
        let second = w.subgroup(group, "English 2").await;
        w.as_admin(
            "PATCH",
            &format!("/subgroups/{second}"),
            Some(json!({"name": " English 1"})),
        )
        .await
        .problem(409);
        let renamed = w
            .as_admin(
                "PATCH",
                &format!("/subgroups/{sub}"),
                Some(json!({"name": "Eng 1"})),
            )
            .await;
        assert_eq!(renamed.json()["name"], "Eng 1");
        let listed = w
            .as_admin("GET", &format!("/groups/{group}/subgroups"), None)
            .await
            .json();
        assert_eq!(ids(&listed, "name"), ["Eng 1", "English 2"]);
    })
    .await;
}

#[tokio::test]
async fn unknown_path_ids_are_404() {
    db_test(async |pool| {
        let w = world(pool).await;
        let unknown = Uuid::now_v7();
        let body = json!({"name": "x"});
        let members = json!({"user_ids": [w.student.id]});
        for (method, path, body) in [
            ("GET", format!("/terms/{unknown}"), None),
            ("DELETE", format!("/terms/{unknown}"), None),
            ("GET", format!("/groups/{unknown}"), None),
            ("PATCH", format!("/groups/{unknown}"), Some(&body)),
            ("DELETE", format!("/groups/{unknown}"), None),
            ("GET", format!("/groups/{unknown}/members"), None),
            ("POST", format!("/groups/{unknown}/members"), Some(&members)),
            (
                "DELETE",
                format!("/groups/{unknown}/members/{}", w.student.id),
                None,
            ),
            ("POST", format!("/groups/{unknown}/subgroups"), Some(&body)),
            ("GET", format!("/groups/{unknown}/subgroups"), None),
            ("PATCH", format!("/subgroups/{unknown}"), Some(&body)),
            ("DELETE", format!("/subgroups/{unknown}"), None),
            ("GET", format!("/subgroups/{unknown}/members"), None),
            (
                "POST",
                format!("/subgroups/{unknown}/members"),
                Some(&members),
            ),
            (
                "DELETE",
                format!("/subgroups/{unknown}/members/{}", w.student.id),
                None,
            ),
        ] {
            let response = call(&w.http, method, &path, Some(&w.admin.token), body).await;
            assert_eq!(response.status, 404, "{method} {path}");
            // A non-admin is refused before the id is looked at.
            let denied = call(&w.http, method, &path, Some(&w.teacher.token), body).await;
            if !path.starts_with("/terms") {
                denied.problem(403);
            }
        }
        let group = w.group("7-A").await;
        w.as_admin(
            "DELETE",
            &format!("/groups/{group}/members/{}", w.student.id),
            None,
        )
        .await
        .problem(404);
    })
    .await;
}

#[tokio::test]
async fn adding_members_is_all_or_nothing_and_idempotent() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let (a, b) = (
            w.new_student("a@x.edu").await,
            w.new_student("b@x.edu").await,
        );

        let first = w.add_to_group(group, &[a, b, a]).await;
        assert_eq!(first.status, 200);
        let mut added: Vec<String> = first.json()["added"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        added.sort();
        let mut expected = vec![a.to_string(), b.to_string()];
        expected.sort();
        assert_eq!(
            added, expected,
            "a repeated id inside one request is added once"
        );

        // A repeat adds nobody, and only the one new student is reported.
        let c = w.new_student("c@x.edu").await;
        let again = w.add_to_group(group, &[a, b, c]).await.json();
        assert_eq!(again["added"], json!([c.to_string()]));
        assert_eq!(
            w.add_to_group(group, &[a, b, c]).await.json()["added"],
            json!([])
        );
        assert_eq!(w.group_member_count(group).await, 3);

        // A teacher, then an unknown user: both are named, nothing is added.
        let d = w.new_student("d@x.edu").await;
        let unknown = Uuid::now_v7();
        let refused = w.add_to_group(group, &[d, w.teacher.id, unknown]).await;
        let problem = refused.problem(422);
        let positions: Vec<_> = problem
            .errors
            .iter()
            .map(|e| (e.field.as_str(), e.message.as_str()))
            .collect();
        assert_eq!(
            positions,
            [
                ("user_ids[1]", "only students can join a group"),
                ("user_ids[2]", "no such user"),
            ]
        );
        assert_eq!(w.group_member_count(group).await, 3, "d was not added");

        for bad in [
            json!({"user_ids": []}),
            json!({"user_ids": ["nope"]}),
            json!({}),
        ] {
            let response = w
                .as_admin("POST", &format!("/groups/{group}/members"), Some(bad))
                .await;
            assert_eq!(response.status, 422);
        }
        let _ = d;
    })
    .await;
}

#[tokio::test]
async fn a_disabled_student_can_be_added_and_a_role_change_keeps_the_membership() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let sleepy = w.new_student("sleepy@x.edu").await;
        let promoted = w.new_student("promoted@x.edu").await;
        w.patch_user(sleepy, json!({"status": "disabled"})).await;

        assert_eq!(w.add_to_group(group, &[sleepy, promoted]).await.status, 200);
        w.patch_user(promoted, json!({"role": "teacher"})).await;
        assert_eq!(w.group_member_count(group).await, 2);

        // The member passes a repeat even though they are a teacher now; a new teacher does not.
        let again = w.add_to_group(group, &[promoted]).await;
        assert_eq!(again.json()["added"], json!([]));
        let teacher = w.add_to_group(group, &[w.teacher.id]).await;
        assert_eq!(fields(&teacher), ["user_ids[0]"]);
    })
    .await;
}

#[tokio::test]
async fn only_members_of_the_group_join_its_subgroups_and_leaving_the_group_leaves_them() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let other_group = w.group("7-B").await;
        let sub = w.subgroup(group, "English 1").await;
        let sub_too = w.subgroup(group, "English 2").await;
        let (a, b, outsider) = (
            w.new_student("a@x.edu").await,
            w.new_student("b@x.edu").await,
            w.new_student("outsider@x.edu").await,
        );
        w.add_to_group(group, &[a, b]).await;
        w.add_to_group(other_group, &[outsider]).await;

        // Positions name the entries that are not in the group, whatever the reason.
        let refused = w.add_to_subgroup(sub, &[a, outsider, Uuid::now_v7()]).await;
        assert_eq!(fields(&refused), ["user_ids[1]", "user_ids[2]"]);
        assert_eq!(w.subgroup_member_count(sub).await, 0, "all or nothing");

        let added = w.add_to_subgroup(sub, &[a, b]).await;
        assert_eq!(added.status, 200);
        assert_eq!(
            w.add_to_subgroup(sub, &[a, b]).await.json()["added"],
            json!([])
        );
        assert_eq!(w.add_to_subgroup(sub_too, &[a]).await.status, 200);
        assert_eq!(w.subgroup_member_count(sub).await, 2);

        // Leaving a subgroup keeps the group membership; leaving the group takes both away.
        w.as_admin("DELETE", &format!("/subgroups/{sub}/members/{b}"), None)
            .await;
        assert_eq!(w.group_member_count(group).await, 2);
        w.as_admin("DELETE", &format!("/subgroups/{sub}/members/{b}"), None)
            .await
            .problem(404);

        assert_eq!(
            w.as_admin("DELETE", &format!("/groups/{group}/members/{a}"), None)
                .await
                .status,
            204
        );
        assert_eq!(w.subgroup_member_count(sub).await, 0);
        assert_eq!(w.subgroup_member_count(sub_too).await, 0);
        assert_eq!(w.group_member_count(group).await, 1);
        let listed = w
            .as_admin("GET", &format!("/subgroups/{sub_too}/members"), None)
            .await
            .json();
        assert!(ids(&listed, "user_id").is_empty());
    })
    .await;
}

#[tokio::test]
async fn member_lists_are_ordered_by_surname_name_and_id_and_paged_by_user_id() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let sub = w.subgroup(group, "English 1").await;
        let mut students = Vec::new();
        for (n, (name, surname)) in [
            ("Zoe", "Adams"),
            ("Anna", "Brown"),
            ("Bob", "Brown"),
            ("Cy", "Clark"),
        ]
        .into_iter()
        .enumerate()
        {
            let id = w.new_student(&format!("s{n}@x.edu")).await;
            w.set_names(id, name, surname).await;
            students.push(id.to_string());
        }
        let all: Vec<Uuid> = students.iter().map(|id| id.parse().unwrap()).collect();
        w.add_to_group(group, &all).await;
        w.add_to_subgroup(sub, &all).await;

        for path in [
            format!("/groups/{group}/members"),
            format!("/subgroups/{sub}/members"),
        ] {
            let first = w
                .as_admin("GET", &format!("{path}?limit=3"), None)
                .await
                .json();
            assert_eq!(ids(&first, "user_id"), students[..3]);
            assert_eq!(first["items"][0]["surname"], "Adams");
            assert_eq!(first["items"][0]["name"], "Zoe");
            let cursor = first["next_cursor"].as_str().unwrap();
            assert_eq!(cursor, students[2]);
            let last = w
                .as_admin("GET", &format!("{path}?limit=3&cursor={cursor}"), None)
                .await
                .json();
            assert_eq!(ids(&last, "user_id"), students[3..]);
            assert_eq!(last["next_cursor"], Value::Null);
            w.as_admin("GET", &format!("{path}?cursor=junk"), None)
                .await
                .problem(422);
        }
    })
    .await;
}

#[tokio::test]
async fn deleting_a_group_deletes_its_subgroups_and_members() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let sub = w.subgroup(group, "English 1").await;
        w.add_to_group(group, &[w.student.id]).await;
        w.add_to_subgroup(sub, &[w.student.id]).await;

        assert_eq!(
            w.as_admin("DELETE", &format!("/groups/{group}"), None)
                .await
                .status,
            204
        );

        w.as_admin("GET", &format!("/subgroups/{sub}/members"), None)
            .await
            .problem(404);
        for table in ["groups", "group_members", "subgroups", "subgroup_members"] {
            let rows: i64 = match table {
                "groups" => sqlx::query_scalar("SELECT count(*) FROM org.groups"),
                "group_members" => sqlx::query_scalar("SELECT count(*) FROM org.group_members"),
                "subgroups" => sqlx::query_scalar("SELECT count(*) FROM org.subgroups"),
                _ => sqlx::query_scalar("SELECT count(*) FROM org.subgroup_members"),
            }
            .fetch_one(&w.pool)
            .await
            .unwrap();
            assert_eq!(rows, 0, "{table}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_term_that_a_course_uses_cannot_be_deleted() {
    db_test(async |pool| {
        let w = world(pool).await;
        let term = w.term("Fall 2026").await;
        let created = call(
            &w.http,
            "POST",
            "/courses",
            Some(&w.teacher.token),
            Some(&json!({"title": "Algebra", "term_id": term})),
        )
        .await;
        assert_eq!(created.status, 201);
        let course = common::id_of(&created.json());

        let refused = w.as_admin("DELETE", &format!("/terms/{term}"), None).await;
        assert_eq!(
            refused.problem(409).detail.as_deref(),
            Some("a course uses this term")
        );
        w.as_admin("GET", &format!("/terms/{term}"), None).await;

        sqlx::query!("DELETE FROM teaching.courses WHERE id = $1", course)
            .execute(&w.pool)
            .await
            .unwrap();
        assert_eq!(
            w.as_admin("DELETE", &format!("/terms/{term}"), None)
                .await
                .status,
            204
        );
    })
    .await;
}

#[tokio::test]
async fn admin_writes_are_audited_and_reads_and_refusals_are_not() {
    db_test(async |pool| {
        let w = world(pool).await;
        let term = w.term("Fall 2026").await;
        w.as_admin(
            "PATCH",
            &format!("/terms/{term}"),
            Some(json!({"name": "Autumn 2026"})),
        )
        .await;
        let group = w.group("7-A").await;
        w.as_admin(
            "PATCH",
            &format!("/groups/{group}"),
            Some(json!({"name": "7-A (2026)"})),
        )
        .await;
        let sub = w.subgroup(group, "English 1").await;
        w.as_admin(
            "PATCH",
            &format!("/subgroups/{sub}"),
            Some(json!({"name": "Eng 1"})),
        )
        .await;
        w.add_to_group(group, &[w.student.id]).await;
        w.add_to_group(group, &[w.student.id]).await; // changes nothing: not audited
        w.add_to_subgroup(sub, &[w.student.id]).await;
        w.as_admin(
            "DELETE",
            &format!("/subgroups/{sub}/members/{}", w.student.id),
            None,
        )
        .await;
        w.as_admin(
            "DELETE",
            &format!("/groups/{group}/members/{}", w.student.id),
            None,
        )
        .await;
        w.as_admin("DELETE", &format!("/subgroups/{sub}"), None)
            .await;
        w.as_admin("DELETE", &format!("/groups/{group}"), None)
            .await;
        w.as_admin("DELETE", &format!("/terms/{term}"), None).await;

        // Reads, refusals and failed writes leave no trace.
        w.as_admin("GET", "/terms", None).await;
        w.as_admin("GET", "/groups", None).await;
        call(
            &w.http,
            "POST",
            "/groups",
            Some(&w.teacher.token),
            Some(&json!({"name": "x"})),
        )
        .await;
        w.as_admin("POST", "/groups", Some(json!({"name": ""})))
            .await;
        w.as_admin("DELETE", &format!("/groups/{}", Uuid::now_v7()), None)
            .await;

        let rows: Vec<(String, String, Uuid, Uuid)> = sqlx::query_as(
            "SELECT action, resource_type, resource_id, actor_id FROM events.audit_log ORDER BY at, id",
        )
        .fetch_all(&w.pool)
        .await
        .unwrap();
        let summary: Vec<_> = rows
            .iter()
            .map(|(a, t, id, _)| (a.as_str(), t.as_str(), *id))
            .collect();
        assert_eq!(
            summary,
            [
                ("term.create", "term", term),
                ("term.update", "term", term),
                ("group.create", "group", group),
                ("group.rename", "group", group),
                ("subgroup.create", "subgroup", sub),
                ("subgroup.rename", "subgroup", sub),
                ("group.member.add", "group", group),
                ("subgroup.member.add", "subgroup", sub),
                ("subgroup.member.remove", "subgroup", sub),
                ("group.member.remove", "group", group),
                ("subgroup.delete", "subgroup", sub),
                ("group.delete", "group", group),
                ("term.delete", "term", term),
            ]
        );
        assert!(rows.iter().all(|(_, _, _, actor)| *actor == w.admin.id));
    })
    .await;
}

#[tokio::test]
async fn every_org_endpoint_is_in_the_openapi_document() {
    db_test(async |pool| {
        let base = common::spawn(pool).await;
        let response = reqwest::get(format!("{base}/api/v1/openapi.json"))
            .await
            .unwrap();
        let doc: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
        for (path, method, tag) in [
            ("/api/v1/terms", "post", "terms"),
            ("/api/v1/terms", "get", "terms"),
            ("/api/v1/terms/{id}", "get", "terms"),
            ("/api/v1/terms/{id}", "patch", "terms"),
            ("/api/v1/terms/{id}", "delete", "terms"),
            ("/api/v1/groups", "post", "groups"),
            ("/api/v1/groups", "get", "groups"),
            ("/api/v1/groups/{id}", "get", "groups"),
            ("/api/v1/groups/{id}", "patch", "groups"),
            ("/api/v1/groups/{id}", "delete", "groups"),
            ("/api/v1/groups/{id}/members", "get", "groups"),
            ("/api/v1/groups/{id}/members", "post", "groups"),
            ("/api/v1/groups/{id}/members/{user_id}", "delete", "groups"),
            ("/api/v1/groups/{id}/subgroups", "post", "groups"),
            ("/api/v1/groups/{id}/subgroups", "get", "groups"),
            ("/api/v1/subgroups/{id}", "patch", "groups"),
            ("/api/v1/subgroups/{id}", "delete", "groups"),
            ("/api/v1/subgroups/{id}/members", "get", "groups"),
            ("/api/v1/subgroups/{id}/members", "post", "groups"),
            (
                "/api/v1/subgroups/{id}/members/{user_id}",
                "delete",
                "groups",
            ),
        ] {
            let operation = &doc["paths"][path][method];
            assert!(operation.is_object(), "{method} {path} missing");
            assert_eq!(operation["tags"], json!([tag]), "{method} {path}");
            assert!(
                operation["security"].is_array(),
                "{method} {path} has no security"
            );
        }
    })
    .await;
}
