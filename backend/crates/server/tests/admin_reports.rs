//! ADR-0053: the administrator's read models and the audit log.

mod common;

use ::common::Role;
use common::{Account, Env, db_test, id_of};
use serde_json::{Value, json};
use uuid::Uuid;

struct World {
    env: Env,
    admin: Account,
    teacher: Account,
    assistant: Account,
    student: Account,
    course: Uuid,
}

async fn world(pool: sqlx::PgPool) -> World {
    let env = Env::new(pool).await;
    let admin = env.user("admin@lms.test", Role::Admin).await;
    let teacher = env.user("teacher@lms.test", Role::Teacher).await;
    let assistant = env.user("assistant@lms.test", Role::Student).await;
    let student = env.user("student@lms.test", Role::Student).await;
    let (course, _) = env.course(&teacher, &[&student]).await;
    env.ok(
        "PUT",
        &teacher.token,
        &format!("/courses/{course}/members/{}", assistant.id),
        Some(json!({"role": "assistant"})),
        200,
    )
    .await;
    World {
        env,
        admin,
        teacher,
        assistant,
        student,
        course,
    }
}

impl World {
    async fn get(&self, who: &Account, path: &str) -> Value {
        self.env.ok("GET", &who.token, path, None, 200).await
    }

    async fn group(&self, name: &str, members: &[&Account]) -> Uuid {
        let group = id_of(
            &self
                .env
                .ok(
                    "POST",
                    &self.admin.token,
                    "/groups",
                    Some(json!({"name": name})),
                    201,
                )
                .await,
        );
        let ids: Vec<Uuid> = members.iter().map(|m| m.id).collect();
        self.env
            .ok(
                "PUT",
                &self.admin.token,
                &format!("/groups/{group}/members"),
                Some(json!({"user_ids": ids})),
                200,
            )
            .await;
        group
    }
}

const ADMIN_PATHS: [&str; 5] = [
    "/admin/users/summary",
    "/admin/term-stats",
    "/admin/courses",
    "/admin/audit",
    "/admin/audit?kind=accounts&days=30",
];

#[tokio::test]
async fn only_an_administrator_reads_the_reports() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("Group A", &[&w.student]).await;
        let mut paths: Vec<String> = ADMIN_PATHS.iter().map(|p| (*p).to_owned()).collect();
        paths.push(format!("/admin/users/{}/facts", w.student.id));
        paths.push(format!("/admin/groups/{group}/overview"));
        for path in &paths {
            assert_eq!(w.env.http.get(path).send().await.status, 401, "{path}");
            for who in [&w.teacher, &w.assistant, &w.student] {
                assert_eq!(
                    w.env.call("GET", &who.token, path, None).await.status,
                    403,
                    "{path}"
                );
            }
            assert_eq!(
                w.env.call("GET", &w.admin.token, path, None).await.status,
                200,
                "{path}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_summary_counts_people_by_role_and_the_disabled() {
    db_test(async |pool| {
        let w = world(pool).await;
        let extra = w.env.user("extra@lms.test", Role::Student).await;
        w.env
            .ok(
                "PATCH",
                &w.admin.token,
                &format!("/admin/users/{}", extra.id),
                Some(json!({"status": "disabled"})),
                200,
            )
            .await;
        let summary = w.get(&w.admin, "/admin/users/summary").await;
        assert_eq!(
            summary,
            json!({"all": 5, "student": 3, "teacher": 1, "admin": 1, "disabled": 1})
        );
    })
    .await;
}

#[tokio::test]
async fn the_facts_of_a_person_name_their_groups_and_courses_and_last_sign_in() {
    db_test(async |pool| {
        let w = world(pool).await;
        w.group("Group A", &[&w.student]).await;
        sqlx::query!("UPDATE teaching.courses SET code = 'CS 1' WHERE id = $1", w.course)
            .execute(&w.env.pool)
            .await
            .unwrap();

        let teacher = w.get(&w.admin, &format!("/admin/users/{}/facts", w.teacher.id)).await;
        assert_eq!(teacher["role"], "teacher");
        assert_eq!(teacher["owns"], json!([{"course_id": w.course, "code": "CS 1", "title": "C"}]));
        assert_eq!(teacher["teaches"], json!([]));
        assert_eq!(teacher["studies"], 0);
        assert!(teacher["last_login_at"].is_string(), "{teacher}");

        let assistant = w.get(&w.admin, &format!("/admin/users/{}/facts", w.assistant.id)).await;
        assert_eq!(assistant["owns"], json!([]));
        assert_eq!(assistant["teaches"][0]["code"], "CS 1");

        let student = w.get(&w.admin, &format!("/admin/users/{}/facts", w.student.id)).await;
        assert_eq!(student["studies"], 1);
        assert_eq!(student["groups"][0]["name"], "Group A");
        assert!(!student.to_string().contains("@lms.test"), "no email: {student}");

        // Someone who has never signed in has no time.
        let created = w
            .env
            .ok(
                "POST",
                &w.admin.token,
                "/admin/users",
                Some(json!({"email": "new@lms.test", "name": "New", "surname": "Person", "role": "student"})),
                201,
            )
            .await;
        let id = created["user"]["id"].as_str().unwrap().to_owned();
        let fresh = w.get(&w.admin, &format!("/admin/users/{id}/facts")).await;
        assert_eq!(fresh["last_login_at"], Value::Null);
        assert_eq!(
            w.env
                .call("GET", &w.admin.token, &format!("/admin/users/{}/facts", Uuid::now_v7()), None)
                .await
                .status,
            404
        );
    })
    .await;
}

#[tokio::test]
async fn a_draft_course_is_not_studied_and_a_template_is_not_owned() {
    db_test(async |pool| {
        let w = world(pool).await;
        let draft = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    "/courses",
                    Some(json!({"title": "Draft"})),
                    201,
                )
                .await,
        );
        w.env
            .ok(
                "PUT",
                &w.teacher.token,
                &format!("/courses/{draft}/members/{}", w.student.id),
                Some(json!({"role": "student"})),
                200,
            )
            .await;
        w.env
            .ok(
                "POST",
                &w.teacher.token,
                &format!("/courses/{}/template", w.course),
                Some(json!({})),
                201,
            )
            .await;
        let student = w
            .get(&w.admin, &format!("/admin/users/{}/facts", w.student.id))
            .await;
        assert_eq!(student["studies"], 1, "only the published course");
        let teacher = w
            .get(&w.admin, &format!("/admin/users/{}/facts", w.teacher.id))
            .await;
        assert_eq!(
            teacher["owns"].as_array().unwrap().len(),
            2,
            "two regular courses, no template"
        );
    })
    .await;
}

#[tokio::test]
async fn the_overview_of_a_group_has_its_people_and_the_courses_that_use_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let other = w.env.user("other@lms.test", Role::Student).await;
        let group = w.group("Group A", &[&w.student, &other]).await;
        w.env
            .ok(
                "POST",
                &w.teacher.token,
                &format!("/courses/{}/groups", w.course),
                Some(json!({"group_id": group})),
                201,
            )
            .await;
        let empty_group = w.group("Empty", &[]).await;

        let overview = w
            .get(&w.admin, &format!("/admin/groups/{group}/overview"))
            .await;
        assert_eq!(overview["group_id"], json!(group));
        assert_eq!(overview["student_count"], 2);
        assert_eq!(overview["course_count"], 1);
        let members = overview["members"].as_array().unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().any(|m| m["email"] == "student@lms.test"));
        assert!(
            members.iter().all(|m| m["added_at"].is_string()),
            "added now, so recorded"
        );
        let linked = &overview["linked_courses"][0];
        assert_eq!(linked["course_id"], json!(w.course));
        assert_eq!(linked["title"], "C");
        assert!(linked["owner_name"].is_string() && linked["owner_surname"].is_string());
        assert!(linked["linked_on"].is_string());

        let empty = w
            .get(&w.admin, &format!("/admin/groups/{empty_group}/overview"))
            .await;
        assert_eq!(empty["student_count"], 0);
        assert_eq!(empty["linked_courses"], json!([]));
        assert_eq!(
            w.env
                .call(
                    "GET",
                    &w.admin.token,
                    &format!("/admin/groups/{}/overview", Uuid::now_v7()),
                    None
                )
                .await
                .status,
            404
        );
    })
    .await;
}

#[tokio::test]
async fn a_member_added_before_the_time_was_recorded_has_none() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("Group A", &[&w.student]).await;
        sqlx::query!(
            "UPDATE org.group_members SET added_at = NULL WHERE group_id = $1",
            group
        )
        .execute(&w.env.pool)
        .await
        .unwrap();
        let overview = w
            .get(&w.admin, &format!("/admin/groups/{group}/overview"))
            .await;
        assert_eq!(overview["members"][0]["added_at"], Value::Null);
    })
    .await;
}

#[tokio::test]
async fn the_term_statistics_count_regular_courses_and_say_where_today_stands() {
    db_test(async |pool| {
        let w = world(pool).await;
        let make = async |name: &str, from: &str, to: &str| -> Uuid {
            id_of(
                &w.env
                    .ok(
                        "POST",
                        &w.admin.token,
                        "/terms",
                        Some(json!({"name": name, "starts_on": from, "ends_on": to})),
                        201,
                    )
                    .await,
            )
        };
        let past = make("Past", "2000-01-01", "2000-06-30").await;
        let now = make("Now", "2000-01-01", "2999-12-31").await;
        let future = make("Future", "2998-01-01", "2999-12-31").await;
        w.env
            .ok(
                "PATCH",
                &w.teacher.token,
                &format!("/courses/{}", w.course),
                Some(json!({"term_id": now})),
                200,
            )
            .await;
        // A template never has a term, so only regular courses can be counted.
        let old = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    "/courses",
                    Some(json!({"title": "Old"})),
                    201,
                )
                .await,
        );
        let older = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    "/courses",
                    Some(json!({"title": "Older"})),
                    201,
                )
                .await,
        );
        for course in [old, older] {
            sqlx::query!(
                "UPDATE teaching.courses SET term_id = $2 WHERE id = $1",
                course,
                past
            )
            .execute(&w.env.pool)
            .await
            .unwrap();
        }

        let stats = w.get(&w.admin, "/admin/term-stats").await;
        let of = |id: Uuid| {
            stats["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["term_id"] == json!(id))
                .cloned()
                .unwrap()
        };
        assert_eq!(
            of(now),
            json!({"term_id": now, "courses": 1, "current": true, "read_only": false})
        );
        assert_eq!(
            of(past),
            json!({"term_id": past, "courses": 2, "current": false, "read_only": true})
        );
        assert_eq!(
            of(future),
            json!({"term_id": future, "courses": 0, "current": false, "read_only": false})
        );
        assert_eq!(stats["items"].as_array().unwrap().len(), 3);
    })
    .await;
}

#[tokio::test]
async fn the_administrator_lists_every_regular_course_in_pages() {
    db_test(async |pool| {
        let w = world(pool).await;
        for title in ["Two", "Three"] {
            w.env
                .ok(
                    "POST",
                    &w.teacher.token,
                    "/courses",
                    Some(json!({"title": title})),
                    201,
                )
                .await;
        }
        w.env
            .ok(
                "POST",
                &w.teacher.token,
                &format!("/courses/{}/template", w.course),
                Some(json!({})),
                201,
            )
            .await;

        let first = w.get(&w.admin, "/admin/courses?limit=2").await;
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        let cursor = first["next_cursor"].as_str().unwrap().to_owned();
        let second = w
            .get(&w.admin, &format!("/admin/courses?limit=2&cursor={cursor}"))
            .await;
        assert_eq!(
            second["items"].as_array().unwrap().len(),
            1,
            "three regular courses, no template"
        );
        assert_eq!(second["next_cursor"], Value::Null);

        let all = w.get(&w.admin, "/admin/courses").await;
        let course = all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["course_id"] == json!(w.course))
            .unwrap();
        assert_eq!(course["status"], "published");
        assert_eq!(course["student_count"], 1);
        assert_eq!(course["owner_id"], json!(w.teacher.id));
        assert!(course["owner_name"].is_string() && course["owner_surname"].is_string());
        assert_eq!(course["term_id"], Value::Null);
    })
    .await;
}

#[tokio::test]
async fn the_audit_log_names_who_did_what_to_what_and_filters_by_kind_and_text() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("Algebra 7-A", &[&w.student]).await;
        let successor = w.env.user("successor@lms.test", Role::Teacher).await;
        w.env
            .ok(
                "PUT",
                &w.admin.token,
                &format!("/courses/{}/owner", w.course),
                Some(json!({"user_id": successor.id})),
                200,
            )
            .await;

        let all = w.get(&w.admin, "/admin/audit").await;
        let items = all["items"].as_array().unwrap();
        assert!(items.len() >= 2, "{all}");
        let created = items
            .iter()
            .find(|e| e["action"] == "group.create")
            .unwrap();
        assert_eq!(created["kind"], "groups");
        assert_eq!(created["source"], "api");
        assert_eq!(created["actor"]["user_id"], json!(w.admin.id));
        assert_eq!(created["resource_type"], "group");
        assert_eq!(created["resource_id"], json!(group));
        assert_eq!(created["resource_label"], "Algebra 7-A");
        let ids: Vec<&str> = items.iter().map(|e| e["id"].as_str().unwrap()).collect();
        let mut newest_first = ids.clone();
        newest_first.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(ids, newest_first, "newest first");

        let groups = w.get(&w.admin, "/admin/audit?kind=groups").await;
        assert!(
            groups["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|e| e["kind"] == "groups")
        );
        let owners = w.get(&w.admin, "/admin/audit?kind=owners").await;
        for entry in owners["items"].as_array().unwrap() {
            assert_eq!(entry["action"], "course.owner.transfer");
        }
        // Text: an action, a target label and an actor's name.
        for q in ["owner.transfer", "algebra", "TEST%20USER"] {
            let found = w.get(&w.admin, &format!("/admin/audit?q={q}")).await;
            assert!(!found["items"].as_array().unwrap().is_empty(), "{q}");
        }
        let none = w.get(&w.admin, "/admin/audit?q=nothing-like-this").await;
        assert_eq!(none["items"], json!([]));
        // A percent sign is a percent sign.
        assert_eq!(
            w.get(&w.admin, "/admin/audit?q=%25").await["items"],
            json!([])
        );
    })
    .await;
}

#[tokio::test]
async fn the_audit_log_pages_limits_the_period_and_survives_deleted_targets() {
    db_test(async |pool| {
        let w = world(pool).await;
        let a = w.group("One", &[]).await;
        w.group("Two", &[]).await;
        w.group("Three", &[]).await;
        w.env
            .ok("DELETE", &w.admin.token, &format!("/groups/{a}"), None, 204)
            .await;

        let first = w.get(&w.admin, "/admin/audit?kind=groups&limit=2").await;
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        let cursor = first["next_cursor"].as_str().unwrap().to_owned();
        let rest = w
            .get(
                &w.admin,
                &format!("/admin/audit?kind=groups&limit=2&cursor={cursor}"),
            )
            .await;
        let seen =
            first["items"].as_array().unwrap().len() + rest["items"].as_array().unwrap().len();
        assert_eq!(seen, 4 + 1 - 1 + 1 - 1, "create x3 + delete, nothing twice");
        assert!(
            !rest["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| first["items"].as_array().unwrap().contains(e))
        );

        let created_one = w.get(&w.admin, "/admin/audit?q=group.create").await["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["resource_id"] == json!(a))
            .cloned()
            .unwrap();
        assert_eq!(
            created_one["resource_label"],
            Value::Null,
            "the group is gone"
        );

        // Rows older than the period are left out.
        sqlx::query!("UPDATE events.audit_log SET at = now() - interval '10 days'")
            .execute(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(w.get(&w.admin, "/admin/audit").await["items"], json!([]));
        assert!(
            !w.get(&w.admin, "/admin/audit?days=30").await["items"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        for bad in [
            "days=0",
            "days=366",
            "kind=nonsense",
            "limit=0",
            "cursor=zzz",
        ] {
            let status = w
                .env
                .call("GET", &w.admin.token, &format!("/admin/audit?{bad}"), None)
                .await
                .status;
            assert_eq!(status, 422, "{bad}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_row_the_server_cli_wrote_has_no_actor() {
    db_test(async |pool| {
        let w = world(pool).await;
        sqlx::query!(
            "INSERT INTO events.audit_log (id, actor_id, source, action, resource_type, resource_id)
             VALUES ($1, NULL, 'cli', 'user.create', 'user', $2)",
            Uuid::now_v7(),
            w.student.id
        )
        .execute(&w.env.pool)
        .await
        .unwrap();
        let entry = &w.get(&w.admin, "/admin/audit?kind=accounts").await["items"][0];
        assert_eq!(entry["source"], "cli");
        assert_eq!(entry["actor"], Value::Null);
        assert_eq!(entry["resource_label"], "Test User");
        assert_eq!(entry["meta"], json!({}));
    })
    .await;
}
