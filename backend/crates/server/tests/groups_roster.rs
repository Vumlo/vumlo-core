//! Group links, the live roster, exclusions and course editing (ADR-0033), over HTTP.

mod common;

use common::{Http, Resp, db_test, id_of, seed_and_login};
use serde_json::{Value, json};
use server::identity::Role;
use sqlx::PgPool;
use uuid::Uuid;

struct User {
    id: Uuid,
    token: String,
}

async fn user(pool: &PgPool, http: &Http, email: &str, role: Role) -> User {
    let (id, token) = seed_and_login(pool, http, email, role).await;
    User { id, token }
}

/// A published course with an owner, an assistant (a student at the institution) and a module
/// with one open assignment; an admin; a teacher who is no member; and a student who is in no
/// group yet.
struct World {
    http: Http,
    pool: PgPool,
    owner: User,
    assistant: User,
    stranger: User,
    admin: User,
    student: User,
    course: Uuid,
    assignment: Uuid,
}

async fn world(pool: PgPool) -> World {
    let http = Http::new(&common::spawn(pool.clone()).await);
    let owner = user(&pool, &http, "owner@x.edu", Role::Teacher).await;
    let assistant = user(&pool, &http, "assistant@x.edu", Role::Student).await;
    let stranger = user(&pool, &http, "stranger@x.edu", Role::Teacher).await;
    let admin = user(&pool, &http, "admin@x.edu", Role::Admin).await;
    let student = user(&pool, &http, "student@x.edu", Role::Student).await;
    let course = common::create_course(&http, &owner.token, "Algebra 7-A").await;
    common::put_member(&http, &owner.token, course, assistant.id, "assistant").await;
    common::publish_course(&http, &owner.token, course).await;
    let mut w = World {
        http,
        pool,
        owner,
        assistant,
        stranger,
        admin,
        student,
        course,
        assignment: Uuid::nil(),
    };
    let module = ok(
        &w.send(
            "POST",
            &w.owner.token,
            &format!("/courses/{course}/modules"),
            Some(json!({"title": "M"})),
        )
        .await,
        201,
    );
    let module = id_of(&module);
    let item = ok(
        &w.send(
            "POST",
            &w.owner.token,
            &format!("/modules/{module}/items"),
            Some(json!({"kind": "assignment", "title": "Quiz",
                        "assignment": {"type": "text", "max_points": "10"}})),
        )
        .await,
        201,
    );
    w.assignment = id_of(&item);
    ok(
        &w.send(
            "POST",
            &w.owner.token,
            &format!("/modules/{module}/publish"),
            Some(json!({"cascade": false})),
        )
        .await,
        200,
    );
    ok(
        &w.send(
            "POST",
            &w.owner.token,
            &format!("/items/{}/publish", w.assignment),
            None,
        )
        .await,
        200,
    );
    w
}

fn ok(response: &Resp, status: u16) -> Value {
    assert_eq!(
        response.status,
        status,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    if response.body.is_empty() {
        Value::Null
    } else {
        response.json()
    }
}

fn fields(response: &Resp) -> Vec<String> {
    response
        .problem(422)
        .errors
        .into_iter()
        .map(|error| error.field)
        .collect()
}

impl World {
    async fn send(&self, method: &str, token: &str, path: &str, body: Option<Value>) -> Resp {
        let request = match method {
            "GET" => self.http.get(path),
            "POST" => self.http.post(path),
            "PUT" => self.http.put(path),
            "PATCH" => self.http.patch(path),
            "DELETE" => self.http.delete(path),
            other => panic!("{other}"),
        }
        .bearer(token);
        match body {
            Some(body) => request.json(&body).send().await,
            None => request.send().await,
        }
    }

    async fn as_admin(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        self.send(method, &self.admin.token, path, body).await
    }

    async fn as_owner(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        self.send(method, &self.owner.token, path, body).await
    }

    async fn new_student(&self, email: &str) -> User {
        user(&self.pool, &self.http, email, Role::Student).await
    }

    async fn group(&self, name: &str) -> Uuid {
        id_of(&ok(
            &self
                .as_admin("POST", "/groups", Some(json!({"name": name})))
                .await,
            201,
        ))
    }

    async fn subgroup(&self, group: Uuid, name: &str) -> Uuid {
        let path = format!("/groups/{group}/subgroups");
        id_of(&ok(
            &self
                .as_admin("POST", &path, Some(json!({"name": name})))
                .await,
            201,
        ))
    }

    async fn join(&self, group: Uuid, users: &[Uuid]) {
        let path = format!("/groups/{group}/members");
        ok(
            &self
                .as_admin("POST", &path, Some(json!({"user_ids": users})))
                .await,
            200,
        );
    }

    async fn join_sub(&self, subgroup: Uuid, users: &[Uuid]) {
        let path = format!("/subgroups/{subgroup}/members");
        ok(
            &self
                .as_admin("POST", &path, Some(json!({"user_ids": users})))
                .await,
            200,
        );
    }

    async fn leave(&self, group: Uuid, user: Uuid) {
        ok(
            &self
                .as_admin("DELETE", &format!("/groups/{group}/members/{user}"), None)
                .await,
            204,
        );
    }

    /// Links the course as its owner; returns the link id.
    async fn link(&self, group: Uuid, subgroup: Option<Uuid>) -> Uuid {
        let path = format!("/courses/{}/groups", self.course);
        let body = json!({"group_id": group, "subgroup_id": subgroup});
        id_of(&ok(&self.as_owner("POST", &path, Some(body)).await, 201))
    }

    /// The roster as `(user_id, role, source)`, by user id.
    async fn roster(&self) -> Vec<(Uuid, String, String)> {
        let path = format!("/courses/{}/members?limit=100", self.course);
        let page = ok(&self.as_owner("GET", &path, None).await, 200);
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                (
                    m["user_id"].as_str().unwrap().parse().unwrap(),
                    m["role"].as_str().unwrap().to_owned(),
                    m["source"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    async fn place_of(&self, user: Uuid) -> Option<(String, String)> {
        self.roster()
            .await
            .into_iter()
            .find(|(id, _, _)| *id == user)
            .map(|(_, role, source)| (role, source))
    }

    async fn status_of_course_for(&self, who: &User) -> u16 {
        self.send(
            "GET",
            &who.token,
            &format!("/courses/{}", self.course),
            None,
        )
        .await
        .status
    }

    async fn my_courses(&self, who: &User) -> Vec<String> {
        let page = ok(
            &self.send("GET", &who.token, "/me/courses", None).await,
            200,
        );
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn gradebook_rows(&self) -> Vec<Uuid> {
        let path = format!("/courses/{}/gradebook?limit=100", self.course);
        let page = ok(&self.as_owner("GET", &path, None).await, 200);
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["user_id"].as_str().unwrap().parse().unwrap())
            .collect()
    }

    /// The users the gradebook lists as former students (ADR-0037).
    async fn former_rows(&self) -> Vec<Uuid> {
        let path = format!("/courses/{}/gradebook?limit=100", self.course);
        let page = ok(&self.as_owner("GET", &path, None).await, 200);
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["former"] == json!(true))
            .map(|row| row["user_id"].as_str().unwrap().parse().unwrap())
            .collect()
    }

    async fn exclusions(&self) -> Vec<Uuid> {
        let path = format!("/courses/{}/exclusions", self.course);
        let page = ok(&self.as_owner("GET", &path, None).await, 200);
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["user_id"].as_str().unwrap().parse().unwrap())
            .collect()
    }

    async fn remove(&self, user: Uuid) -> Resp {
        self.as_owner(
            "DELETE",
            &format!("/courses/{}/members/{user}", self.course),
            None,
        )
        .await
    }

    async fn direct_rows(&self, user: Uuid) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM teaching.memberships WHERE course_id = $1 AND user_id = $2"#,
            self.course,
            user
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn grades_of(&self, user: Uuid) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM gradebook.grades WHERE student_id = $1"#,
            user
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }

    async fn link_rows(&self) -> i64 {
        sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM teaching.course_groups"#)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn audit_rows(&self) -> i64 {
        sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.audit_log"#)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn set_status(&self, course: Uuid, status: &str) {
        sqlx::query!(
            "UPDATE teaching.courses SET status = $2 WHERE id = $1",
            course,
            status
        )
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn make_template(&self, course: Uuid) {
        sqlx::query!(
            "UPDATE teaching.courses SET kind = 'template' WHERE id = $1",
            course
        )
        .execute(&self.pool)
        .await
        .unwrap();
    }
}

// --- the live roster ---------------------------------------------------------------------------

#[tokio::test]
async fn a_student_of_a_linked_group_sees_the_published_course_and_loses_it_on_leaving() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let s = w.new_student("s@x.edu").await;
        w.join(group, &[s.id]).await;
        let draft = common::create_course(&w.http, &w.owner.token, "Draft").await;

        assert_eq!(w.status_of_course_for(&s).await, 404, "not linked yet");
        w.link(group, None).await;
        assert_eq!(w.status_of_course_for(&s).await, 200);
        assert!(w.my_courses(&s).await.contains(&w.course.to_string()));
        assert_eq!(
            w.place_of(s.id).await,
            Some(("student".into(), "group".into()))
        );

        // A draft course stays invisible to a student of a linked group, as to any student.
        let link = json!({"group_id": group});
        ok(
            &w.as_owner("POST", &format!("/courses/{draft}/groups"), Some(link))
                .await,
            201,
        );
        let hidden = w
            .send("GET", &s.token, &format!("/courses/{draft}"), None)
            .await;
        hidden.problem(404);
        assert!(!w.my_courses(&s).await.contains(&draft.to_string()));

        w.leave(group, s.id).await;
        assert_eq!(w.status_of_course_for(&s).await, 404);
        assert!(w.my_courses(&s).await.is_empty());
        assert_eq!(w.place_of(s.id).await, None);
    })
    .await;
}

#[tokio::test]
async fn a_subgroup_link_brings_only_that_subgroup() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let sub = w.subgroup(group, "English 1").await;
        let (a, b) = (
            w.new_student("a@x.edu").await,
            w.new_student("b@x.edu").await,
        );
        w.join(group, &[a.id, b.id]).await;
        w.join_sub(sub, &[a.id]).await;

        w.link(group, Some(sub)).await;

        assert_eq!(
            w.place_of(a.id).await,
            Some(("student".into(), "group".into()))
        );
        assert_eq!(w.place_of(b.id).await, None);
        assert_eq!(w.status_of_course_for(&a).await, 200);
        assert_eq!(w.status_of_course_for(&b).await, 404);
        // Leaving the subgroup (not the group) takes the student off.
        ok(
            &w.as_admin(
                "DELETE",
                &format!("/subgroups/{sub}/members/{}", a.id),
                None,
            )
            .await,
            204,
        );
        assert_eq!(w.status_of_course_for(&a).await, 404);
    })
    .await;
}

#[tokio::test]
async fn two_links_to_the_same_user_are_one_roster_row() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let sub = w.subgroup(group, "English 1").await;
        let other = w.group("7-B").await;
        let s = w.new_student("s@x.edu").await;
        w.join(group, &[s.id]).await;
        w.join(other, &[s.id]).await;
        w.join_sub(sub, &[s.id]).await;
        w.link(group, None).await;
        w.link(group, Some(sub)).await;
        w.link(other, None).await;

        let mine: Vec<_> = w
            .roster()
            .await
            .into_iter()
            .filter(|(id, _, _)| *id == s.id)
            .collect();
        assert_eq!(mine.len(), 1);
        assert_eq!(
            w.gradebook_rows()
                .await
                .iter()
                .filter(|id| **id == s.id)
                .count(),
            1
        );
        // The student leaves one group: the others still hold them.
        w.leave(other, s.id).await;
        assert!(w.place_of(s.id).await.is_some());
    })
    .await;
}

#[tokio::test]
async fn a_direct_membership_wins_over_a_group() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        w.join(group, &[w.assistant.id]).await;
        w.link(group, None).await;

        // The direct assistant is also a group student: the assistant role and the direct source win.
        assert_eq!(
            w.place_of(w.assistant.id).await,
            Some(("assistant".into(), "direct".into()))
        );
        let drafts = w
            .send(
                "GET",
                &w.assistant.token,
                &format!("/courses/{}/gradebook", w.course),
                None,
            )
            .await;
        assert_eq!(drafts.status, 200, "still staff");
    })
    .await;
}

// --- exclusions --------------------------------------------------------------------------------

#[tokio::test]
async fn removing_a_group_student_excludes_them_and_restoring_brings_them_back() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let s = w.new_student("s@x.edu").await;
        w.join(group, &[s.id]).await;
        w.link(group, None).await;
        let grade = format!("/assignments/{}/grades/{}", w.assignment, s.id);
        ok(
            &w.as_owner("PUT", &grade, Some(json!({"score": "7", "feedback": null})))
                .await,
            200,
        );
        assert!(w.gradebook_rows().await.contains(&s.id));

        assert_eq!(w.remove(s.id).await.status, 204);

        assert_eq!(w.status_of_course_for(&s).await, 404, "no access");
        assert_eq!(w.place_of(s.id).await, None);
        assert_eq!(
            w.former_rows().await,
            [s.id],
            "off the roster, but the kept grade makes them a former student"
        );
        assert_eq!(w.exclusions().await, [s.id]);
        let kept = w.grades_of(s.id).await;
        assert_eq!(kept, 1, "the grade stays");
        let listed = ok(
            &w.as_owner("GET", &format!("/courses/{}/exclusions", w.course), None)
                .await,
            200,
        );
        assert_eq!(listed["items"][0]["created_by"], w.owner.id.to_string());

        ok(
            &w.as_owner(
                "DELETE",
                &format!("/courses/{}/exclusions/{}", w.course, s.id),
                None,
            )
            .await,
            204,
        );
        assert_eq!(w.status_of_course_for(&s).await, 200);
        let row = w.gradebook_rows().await;
        assert!(row.contains(&s.id));
        assert!(w.former_rows().await.is_empty(), "back on the roster");
        let book = ok(
            &w.as_owner("GET", &format!("/courses/{}/gradebook", w.course), None)
                .await,
            200,
        );
        let cell = &book["items"][0]["cells"][0];
        assert_eq!(cell["draft_score"], "7.00", "{book}");
        w.as_owner(
            "DELETE",
            &format!("/courses/{}/exclusions/{}", w.course, s.id),
            None,
        )
        .await
        .problem(404);
    })
    .await;
}

#[tokio::test]
async fn removing_a_direct_student_excludes_only_when_a_group_would_bring_them_back() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let (grouped, lone) = (
            w.new_student("g@x.edu").await,
            w.new_student("l@x.edu").await,
        );
        // The direct membership comes first: once a linked group puts a student on the roster, a put
        // as `student` changes nothing, so no direct row would be written.
        for s in [&grouped, &lone] {
            common::put_member(&w.http, &w.owner.token, w.course, s.id, "student").await;
        }
        w.join(group, &[grouped.id]).await;
        w.link(group, None).await;
        assert_eq!(
            w.place_of(grouped.id).await,
            Some(("student".into(), "direct".into()))
        );

        assert_eq!(w.remove(grouped.id).await.status, 204);
        assert_eq!(w.remove(lone.id).await.status, 204);

        assert_eq!(
            w.exclusions().await,
            [grouped.id],
            "only the one a group would bring back"
        );
        assert_eq!(w.place_of(grouped.id).await, None);
        assert_eq!(w.place_of(lone.id).await, None);
        assert_eq!(
            w.direct_rows(grouped.id).await + w.direct_rows(lone.id).await,
            0
        );
        w.remove(lone.id).await.problem(404);
    })
    .await;
}

#[tokio::test]
async fn removing_a_direct_assistant_never_excludes_but_a_linked_group_keeps_them_as_a_student() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        w.join(group, &[w.assistant.id]).await;
        w.link(group, None).await;

        assert_eq!(w.remove(w.assistant.id).await.status, 204);

        assert!(
            w.exclusions().await.is_empty(),
            "no exclusion for a teacher or an assistant"
        );
        assert_eq!(
            w.place_of(w.assistant.id).await,
            Some(("student".into(), "group".into()))
        );
        // ...which is also why the assistant is a student of the course now and cannot grade.
        let book = w
            .send(
                "GET",
                &w.assistant.token,
                &format!("/courses/{}/gradebook", w.course),
                None,
            )
            .await;
        assert_eq!(book.status, 403);
    })
    .await;
}

#[tokio::test]
async fn adding_a_direct_membership_clears_the_exclusion() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let s = w.new_student("s@x.edu").await;
        w.join(group, &[s.id]).await;
        let link = w.link(group, None).await;
        assert_eq!(w.remove(s.id).await.status, 204);
        assert_eq!(w.exclusions().await, [s.id]);

        let put = w
            .as_owner(
                "PUT",
                &format!("/courses/{}/members/{}", w.course, s.id),
                Some(json!({"role": "student"})),
            )
            .await;
        assert_eq!(ok(&put, 200)["source"], "direct");

        assert!(w.exclusions().await.is_empty());
        // The direct membership holds without the group.
        ok(
            &w.as_owner(
                "DELETE",
                &format!("/courses/{}/groups/{link}", w.course),
                None,
            )
            .await,
            204,
        );
        assert_eq!(
            w.place_of(s.id).await,
            Some(("student".into(), "direct".into()))
        );
    })
    .await;
}

#[tokio::test]
async fn an_exclusion_outlives_the_group_membership() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let s = w.new_student("s@x.edu").await;
        w.join(group, &[s.id]).await;
        w.link(group, None).await;
        assert_eq!(w.remove(s.id).await.status, 204);

        // Leaving the group leaves the exclusion in place, where it does nothing.
        w.leave(group, s.id).await;
        assert_eq!(w.exclusions().await, [s.id]);
        // Joining again does not bring them back: the exclusion is still in force.
        w.join(group, &[s.id]).await;
        assert_eq!(w.place_of(s.id).await, None);

        ok(
            &w.as_owner(
                "DELETE",
                &format!("/courses/{}/exclusions/{}", w.course, s.id),
                None,
            )
            .await,
            204,
        );
        assert_eq!(
            w.place_of(s.id).await,
            Some(("student".into(), "group".into()))
        );
    })
    .await;
}

#[tokio::test]
async fn a_group_student_put_as_student_changes_nothing_but_another_role_writes_a_direct_row() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let s = w.new_student("s@x.edu").await;
        w.join(group, &[s.id]).await;
        w.link(group, None).await;
        let path = format!("/courses/{}/members/{}", w.course, s.id);

        let same = ok(
            &w.as_owner("PUT", &path, Some(json!({"role": "student"})))
                .await,
            200,
        );
        assert_eq!(same["source"], "group");
        assert_eq!(w.direct_rows(s.id).await, 0);

        let promoted = ok(
            &w.as_owner("PUT", &path, Some(json!({"role": "assistant"})))
                .await,
            200,
        );
        assert_eq!(promoted["source"], "direct");
        assert_eq!(
            w.place_of(s.id).await,
            Some(("assistant".into(), "direct".into()))
        );
    })
    .await;
}

#[tokio::test]
async fn exclusions_are_listed_in_pages_by_user_id() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let mut ids = Vec::new();
        for n in 0..3 {
            let s = w.new_student(&format!("s{n}@x.edu")).await;
            ids.push(s.id);
        }
        w.join(group, &ids).await;
        w.link(group, None).await;
        for id in &ids {
            assert_eq!(w.remove(*id).await.status, 204);
        }
        ids.sort();
        let path = format!("/courses/{}/exclusions", w.course);
        let first = ok(
            &w.as_owner("GET", &format!("{path}?limit=2"), None).await,
            200,
        );
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        let cursor = first["next_cursor"].as_str().unwrap().to_owned();
        assert_eq!(cursor, ids[1].to_string());
        let last = ok(
            &w.as_owner("GET", &format!("{path}?limit=2&cursor={cursor}"), None)
                .await,
            200,
        );
        assert_eq!(last["items"][0]["user_id"], ids[2].to_string());
        assert_eq!(last["next_cursor"], Value::Null);
        w.as_owner("GET", &format!("{path}?cursor=junk"), None)
            .await
            .problem(422);
    })
    .await;
}

// --- links -------------------------------------------------------------------------------------

#[tokio::test]
async fn linking_is_idempotent_and_names_a_real_group_and_one_of_its_subgroups() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let other = w.group("7-B").await;
        let sub = w.subgroup(group, "English 1").await;
        let path = format!("/courses/{}/groups", w.course);

        let first = w
            .as_owner("POST", &path, Some(json!({"group_id": group})))
            .await;
        let created = ok(&first, 201);
        assert_eq!(created["subgroup_id"], Value::Null);
        let again = ok(
            &w.as_owner("POST", &path, Some(json!({"group_id": group})))
                .await,
            200,
        );
        assert_eq!(again["id"], created["id"]);
        let sub_link = ok(
            &w.as_owner(
                "POST",
                &path,
                Some(json!({"group_id": group, "subgroup_id": sub})),
            )
            .await,
            201,
        );
        assert_eq!(
            ok(
                &w.as_owner(
                    "POST",
                    &path,
                    Some(json!({"group_id": group, "subgroup_id": sub}))
                )
                .await,
                200
            )["id"],
            sub_link["id"]
        );

        let unknown_group = w
            .as_owner("POST", &path, Some(json!({"group_id": Uuid::now_v7()})))
            .await;
        assert_eq!(fields(&unknown_group), ["group_id"]);
        let unknown_sub = w
            .as_owner(
                "POST",
                &path,
                Some(json!({"group_id": group, "subgroup_id": Uuid::now_v7()})),
            )
            .await;
        assert_eq!(fields(&unknown_sub), ["subgroup_id"]);
        let foreign = w
            .as_owner(
                "POST",
                &path,
                Some(json!({"group_id": other, "subgroup_id": sub})),
            )
            .await;
        assert_eq!(fields(&foreign), ["subgroup_id"]);

        let listed = ok(&w.as_owner("GET", &path, None).await, 200);
        assert_eq!(listed["items"].as_array().unwrap().len(), 2);
        let link = created["id"].as_str().unwrap();
        ok(
            &w.as_owner("DELETE", &format!("{path}/{link}"), None).await,
            204,
        );
        w.as_owner("DELETE", &format!("{path}/{link}"), None)
            .await
            .problem(404);
        let remaining = ok(&w.as_owner("GET", &path, None).await, 200);
        assert_eq!(remaining["items"].as_array().unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_link_of_another_course_cannot_be_removed_through_this_one() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let link = w.link(group, None).await;
        let other = common::create_course(&w.http, &w.owner.token, "Other").await;

        w.as_owner("DELETE", &format!("/courses/{other}/groups/{link}"), None)
            .await
            .problem(404);
        assert_eq!(w.link_rows().await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_linked_group_or_subgroup_cannot_be_deleted_until_it_is_unlinked() {
    db_test(async |pool| {
        let w = world(pool).await;
        let whole = w.group("7-A").await;
        let part = w.group("7-B").await;
        let sub = w.subgroup(part, "English 1").await;
        let whole_link = w.link(whole, None).await;
        let sub_link = w.link(part, Some(sub)).await;

        assert_eq!(
            w.as_admin("DELETE", &format!("/groups/{whole}"), None)
                .await
                .problem(409)
                .detail
                .as_deref(),
            Some("a course uses this group")
        );
        assert_eq!(
            w.as_admin("DELETE", &format!("/subgroups/{sub}"), None)
                .await
                .problem(409)
                .detail
                .as_deref(),
            Some("a course uses this subgroup")
        );
        // A group whose subgroup is linked is held by that link.
        w.as_admin("DELETE", &format!("/groups/{part}"), None)
            .await
            .problem(409);

        for link in [whole_link, sub_link] {
            ok(
                &w.as_owner(
                    "DELETE",
                    &format!("/courses/{}/groups/{link}", w.course),
                    None,
                )
                .await,
                204,
            );
        }
        ok(
            &w.as_admin("DELETE", &format!("/subgroups/{sub}"), None)
                .await,
            204,
        );
        ok(
            &w.as_admin("DELETE", &format!("/groups/{whole}"), None)
                .await,
            204,
        );
        ok(
            &w.as_admin("DELETE", &format!("/groups/{part}"), None).await,
            204,
        );
    })
    .await;
}

#[tokio::test]
async fn the_group_options_give_staff_groups_and_subgroups_without_member_data() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let sub = w.subgroup(group, "English 1").await;
        let second = w.group("7-B").await;
        let s = w.new_student("s@x.edu").await;
        w.join(group, &[s.id]).await;
        let path = format!("/courses/{}/group-options", w.course);

        // An assistant manages the roster but does not link groups, so does not choose one (ADR-0040).
        w.send("GET", &w.assistant.token, &path, None)
            .await
            .problem(403);
        for who in [&w.owner, &w.admin] {
            let page = ok(&w.send("GET", &who.token, &path, None).await, 200);
            let items = page["items"].as_array().unwrap();
            assert_eq!(items.len(), 2);
            let mut keys: Vec<_> = items[0].as_object().unwrap().keys().cloned().collect();
            keys.sort();
            assert_eq!(keys, ["id", "name", "subgroups"]);
            assert_eq!(items[0]["name"], "7-A");
            let mut sub_keys: Vec<_> = items[0]["subgroups"][0]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            sub_keys.sort();
            assert_eq!(sub_keys, ["id", "name"]);
            assert_eq!(items[0]["subgroups"][0]["id"], sub.to_string());
            assert_eq!(items[1]["subgroups"], json!([]));
            assert!(
                !page.to_string().contains(&s.id.to_string()),
                "no member data"
            );
        }
        // Paged by group id.
        let first = ok(
            &w.as_owner("GET", &format!("{path}?limit=1"), None).await,
            200,
        );
        assert_eq!(first["items"][0]["id"], group.to_string());
        let last = ok(
            &w.as_owner("GET", &format!("{path}?limit=1&cursor={group}"), None)
                .await,
            200,
        );
        assert_eq!(last["items"][0]["id"], second.to_string());
        assert_eq!(last["next_cursor"], Value::Null);
        // org's own group endpoints stay admin only.
        w.send("GET", &w.owner.token, "/groups", None)
            .await
            .problem(403);
    })
    .await;
}

#[tokio::test]
async fn who_may_manage_links_exclusions_and_the_options() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let draft = common::create_course(&w.http, &w.owner.token, "Draft").await;
        let member = w.new_student("m@x.edu").await;
        common::put_member(&w.http, &w.owner.token, w.course, member.id, "student").await;
        let course = w.course;
        // The last field: may an assistant do it? Linking and choosing a group are for the owner,
        // teachers and admins (ADR-0040); listing the links and the exclusions are roster work.
        let calls: Vec<(&str, String, Option<Value>, bool)> = vec![
            ("GET", format!("/courses/{course}/groups"), None, true),
            (
                "POST",
                format!("/courses/{course}/groups"),
                Some(json!({"group_id": group})),
                false,
            ),
            (
                "GET",
                format!("/courses/{course}/group-options"),
                None,
                false,
            ),
            ("GET", format!("/courses/{course}/exclusions"), None, true),
        ];
        for (method, path, body, assistant_may) in &calls {
            let anonymous = match body {
                Some(body) => w.http.post(path).json(body).send().await,
                None => w.http.get(path).send().await,
            };
            assert_eq!(anonymous.status, 401, "{method} {path}");
            // A student of the published course: 403. A teacher who is no member: 404.
            w.send(method, &member.token, path, body.clone())
                .await
                .problem(403);
            w.send(method, &w.stranger.token, path, body.clone())
                .await
                .problem(404);
            for staff in [&w.owner, &w.admin] {
                let status = w
                    .send(method, &staff.token, path, body.clone())
                    .await
                    .status;
                assert!(status == 200 || status == 201, "{method} {path}: {status}");
            }
            let status = w
                .send(method, &w.assistant.token, path, body.clone())
                .await
                .status;
            if *assistant_may {
                assert_eq!(status, 200, "{method} {path}");
            } else {
                assert_eq!(status, 403, "{method} {path}");
            }
        }
        // The same students get 404, not 403, in a course they cannot see (a draft).
        let hidden = common::create_course(&w.http, &w.owner.token, "Hidden").await;
        common::put_member(&w.http, &w.owner.token, hidden, member.id, "student").await;
        w.send(
            "GET",
            &member.token,
            &format!("/courses/{hidden}/group-options"),
            None,
        )
        .await
        .problem(404);
        w.send(
            "GET",
            &member.token,
            &format!("/courses/{draft}/exclusions"),
            None,
        )
        .await
        .problem(404);
        // A student cannot unlink or restore either.
        let link = id_of(&ok(
            &w.as_owner(
                "POST",
                &format!("/courses/{course}/groups"),
                Some(json!({"group_id": group})),
            )
            .await,
            200,
        ));
        w.send(
            "DELETE",
            &member.token,
            &format!("/courses/{course}/groups/{link}"),
            None,
        )
        .await
        .problem(403);
        w.send(
            "DELETE",
            &w.assistant.token,
            &format!("/courses/{course}/groups/{link}"),
            None,
        )
        .await
        .problem(403);
        w.send(
            "DELETE",
            &member.token,
            &format!("/courses/{course}/exclusions/{}", w.student.id),
            None,
        )
        .await
        .problem(403);
        w.http
            .delete(&format!("/courses/{course}/groups/{link}"))
            .send()
            .await
            .problem(401);
    })
    .await;
}

// --- editing -----------------------------------------------------------------------------------

async fn term(w: &World, name: &str) -> Uuid {
    let body = json!({"name": name, "starts_on": "2026-09-01", "ends_on": "2027-01-31"});
    id_of(&ok(&w.as_admin("POST", "/terms", Some(body)).await, 201))
}

#[tokio::test]
async fn staff_and_admins_edit_the_title_and_the_term_and_students_cannot() {
    db_test(async |pool| {
        let w = world(pool).await;
        let fall = term(&w, "Fall").await;
        let spring = term(&w, "Spring").await;
        let path = format!("/courses/{}", w.course);

        let titled = ok(
            &w.send(
                "PATCH",
                &w.assistant.token,
                &path,
                Some(json!({"title": "  Algebra 7-B "})),
            )
            .await,
            200,
        );
        assert_eq!(titled["title"], "Algebra 7-B");
        assert_eq!(titled["term_id"], Value::Null);
        let termed = ok(
            &w.as_owner("PATCH", &path, Some(json!({"term_id": fall})))
                .await,
            200,
        );
        assert_eq!(termed["term_id"], fall.to_string());
        assert_eq!(
            termed["title"], "Algebra 7-B",
            "an absent field changes nothing"
        );
        let moved = ok(
            &w.as_admin(
                "PATCH",
                &path,
                Some(json!({"title": "Algebra", "term_id": spring})),
            )
            .await,
            200,
        );
        assert_eq!(
            (moved["title"].as_str(), moved["term_id"].as_str()),
            (Some("Algebra"), Some(spring.to_string().as_str()))
        );
        let cleared = ok(
            &w.as_owner("PATCH", &path, Some(json!({"term_id": null})))
                .await,
            200,
        );
        assert_eq!(cleared["term_id"], Value::Null);
        assert_eq!(cleared["title"], "Algebra");
        let empty = ok(&w.as_owner("PATCH", &path, Some(json!({}))).await, 200);
        assert_eq!(empty["title"], "Algebra");

        // A student of the published course: 403. A teacher who is no member: 404.
        w.send(
            "PATCH",
            &w.student.token,
            &path,
            Some(json!({"title": "x"})),
        )
        .await
        .problem(404);
        let member = w.new_student("m@x.edu").await;
        common::put_member(&w.http, &w.owner.token, w.course, member.id, "student").await;
        w.send("PATCH", &member.token, &path, Some(json!({"title": "x"})))
            .await
            .problem(403);
        w.send(
            "PATCH",
            &w.stranger.token,
            &path,
            Some(json!({"title": "x"})),
        )
        .await
        .problem(404);
        w.http
            .patch(&path)
            .json(&json!({"title": "x"}))
            .send()
            .await
            .problem(401);
        let unchanged = ok(&w.as_owner("GET", &path, None).await, 200);
        assert_eq!(unchanged["title"], "Algebra");
    })
    .await;
}

#[tokio::test]
async fn a_course_is_edited_when_it_is_a_draft_or_published_but_not_when_archived_and_the_edit_is_checked()
 {
    db_test(async |pool| {
        let w = world(pool).await;
        let fall = term(&w, "Fall").await;
        let draft = common::create_course(&w.http, &w.owner.token, "Draft").await;
        for (course, status) in [(draft, "draft"), (w.course, "published")] {
            w.set_status(course, status).await;
            let path = format!("/courses/{course}");
            ok(
                &w.as_owner("PATCH", &path, Some(json!({"title": "Renamed"})))
                    .await,
                200,
            );
        }
        // An archived course takes no edit (ADR-0048), and the refusal says why.
        w.set_status(w.course, "archived").await;
        let refused = w
            .as_owner(
                "PATCH",
                &format!("/courses/{}", w.course),
                Some(json!({"title": "Again"})),
            )
            .await;
        assert_eq!(
            refused.problem(409).detail.as_deref(),
            Some("the course is archived")
        );
        let unchanged = ok(
            &w.as_owner("GET", &format!("/courses/{}", w.course), None)
                .await,
            200,
        );
        assert_eq!(unchanged["title"], "Renamed");

        let path = format!("/courses/{draft}");
        for bad in [
            json!({"title": ""}),
            json!({"title": "x".repeat(201)}),
            json!({"title": "a\nb"}),
        ] {
            let response = w.as_owner("PATCH", &path, Some(bad)).await;
            assert_eq!(fields(&response), ["title"]);
        }
        let unknown = w
            .as_owner("PATCH", &path, Some(json!({"term_id": Uuid::now_v7()})))
            .await;
        assert_eq!(fields(&unknown), ["term_id"]);
        w.as_owner("PATCH", &path, Some(json!({"term_id": 5})))
            .await
            .problem(422);

        // A template has no term, but it can still be renamed and its term cleared.
        w.make_template(draft).await;
        let on_template = w
            .as_owner("PATCH", &path, Some(json!({"term_id": fall})))
            .await;
        assert_eq!(fields(&on_template), ["term_id"]);
        ok(
            &w.as_owner(
                "PATCH",
                &path,
                Some(json!({"term_id": null, "title": "Template"})),
            )
            .await,
            200,
        );
    })
    .await;
}

#[tokio::test]
async fn admin_roster_and_edit_writes_are_audited_and_staff_writes_are_not() {
    db_test(async |pool| {
        let w = world(pool).await;
        let group = w.group("7-A").await;
        let s = w.new_student("s@x.edu").await;
        w.join(group, &[s.id]).await;
        let fall = term(&w, "Fall").await;
        let audit_before = w.audit_rows().await;

        // Staff writes leave nothing in the audit log.
        let link = w.link(group, None).await;
        assert_eq!(w.remove(s.id).await.status, 204);
        ok(
            &w.as_owner(
                "DELETE",
                &format!("/courses/{}/exclusions/{}", w.course, s.id),
                None,
            )
            .await,
            204,
        );
        ok(
            &w.as_owner(
                "PATCH",
                &format!("/courses/{}", w.course),
                Some(json!({"title": "Teacher"})),
            )
            .await,
            200,
        );
        ok(
            &w.as_owner(
                "DELETE",
                &format!("/courses/{}/groups/{link}", w.course),
                None,
            )
            .await,
            204,
        );
        assert_eq!(w.audit_rows().await, audit_before);

        // The admin's are.
        let path = format!("/courses/{}", w.course);
        let linked = ok(
            &w.as_admin(
                "POST",
                &format!("{path}/groups"),
                Some(json!({"group_id": group})),
            )
            .await,
            201,
        );
        assert_eq!(
            w.as_admin("DELETE", &format!("{path}/members/{}", s.id), None)
                .await
                .status,
            204
        );
        ok(
            &w.as_admin("DELETE", &format!("{path}/exclusions/{}", s.id), None)
                .await,
            204,
        );
        ok(
            &w.as_admin(
                "PATCH",
                &path,
                Some(json!({"title": "Admin", "term_id": fall})),
            )
            .await,
            200,
        );
        ok(&w.as_admin("PATCH", &path, Some(json!({}))).await, 200);
        ok(
            &w.as_admin(
                "DELETE",
                &format!("{path}/groups/{}", linked["id"].as_str().unwrap()),
                None,
            )
            .await,
            204,
        );

        let rows = sqlx::query!(
            "SELECT action, resource_id, meta FROM events.audit_log
             WHERE resource_type = 'course' AND action <> 'course.create'
               AND actor_id = $1 ORDER BY at, id",
            w.admin.id
        )
        .fetch_all(&w.pool)
        .await
        .unwrap();
        let actions: Vec<_> = rows
            .iter()
            .map(|row| (row.action.as_str(), row.resource_id))
            .collect();
        assert_eq!(
            actions,
            [
                ("course.group.link", w.course),
                ("course.member.remove", w.course),
                ("course.exclusion.restore", w.course),
                ("course.update", w.course),
                ("course.group.unlink", w.course),
            ]
        );
        assert_eq!(
            rows[0].meta,
            json!({"group_id": group.to_string(), "subgroup_id": null})
        );
        assert_eq!(
            rows[1].meta,
            json!({"user_id": s.id.to_string(), "role": "student", "excluded": true})
        );
        assert_eq!(rows[2].meta, json!({"user_id": s.id.to_string()}));
        assert_eq!(rows[3].meta, json!({"fields": ["title", "term_id"]}));
    })
    .await;
}

#[tokio::test]
async fn every_new_course_endpoint_is_in_the_openapi_document() {
    db_test(async |pool| {
        let base = common::spawn(pool).await;
        let response = reqwest::get(format!("{base}/api/v1/openapi.json"))
            .await
            .unwrap();
        let doc: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
        for (path, method) in [
            ("/api/v1/courses/{id}", "patch"),
            ("/api/v1/courses/{id}/groups", "get"),
            ("/api/v1/courses/{id}/groups", "post"),
            ("/api/v1/courses/{id}/groups/{link_id}", "delete"),
            ("/api/v1/courses/{id}/group-options", "get"),
            ("/api/v1/courses/{id}/exclusions", "get"),
            ("/api/v1/courses/{id}/exclusions/{user_id}", "delete"),
        ] {
            let operation = &doc["paths"][path][method];
            assert!(operation.is_object(), "{method} {path} missing");
            assert_eq!(operation["tags"], json!(["courses"]), "{method} {path}");
            assert!(
                operation["security"].is_array(),
                "{method} {path} has no security"
            );
        }
        let member = &doc["components"]["schemas"]["Membership"]["properties"];
        assert!(member["source"].is_object(), "{member}");
    })
    .await;
}
