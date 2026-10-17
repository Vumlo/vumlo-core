mod common;

use common::{Http, Resp, db_test, seed_and_login};
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

async fn setup(pool: &PgPool) -> Http {
    Http::new(&common::spawn(pool.clone()).await)
}

/// A draft course with an owner (teacher), an assistant and a student already added.
struct World {
    http: Http,
    pool: PgPool,
    owner: User,
    assistant: User,
    student: User,
    course: Uuid,
}

async fn world(pool: PgPool) -> World {
    let http = setup(&pool).await;
    let owner = user(&pool, &http, "owner@x.edu", Role::Teacher).await;
    let assistant = user(&pool, &http, "assistant@x.edu", Role::Student).await;
    let student = user(&pool, &http, "student@x.edu", Role::Student).await;
    let course = create_course(&http, &owner.token, "Algebra 7-A").await;
    assert_eq!(
        put_member(&http, &owner.token, course, assistant.id, "assistant")
            .await
            .status,
        200
    );
    assert_eq!(
        put_member(&http, &owner.token, course, student.id, "student")
            .await
            .status,
        200
    );
    World {
        http,
        pool,
        owner,
        assistant,
        student,
        course,
    }
}

async fn create_course(http: &Http, token: &str, title: &str) -> Uuid {
    let response = http
        .post("/courses")
        .bearer(token)
        .json(&json!({"title": title}))
        .send()
        .await;
    assert_eq!(
        response.status,
        201,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    id_of(&response.json())
}

fn id_of(value: &Value) -> Uuid {
    Uuid::parse_str(value["id"].as_str().expect("id")).expect("uuid")
}

async fn get(http: &Http, token: &str, path: &str) -> Resp {
    http.get(path).bearer(token).send().await
}

async fn put_member(http: &Http, token: &str, course: Uuid, user: Uuid, role: &str) -> Resp {
    http.put(&format!("/courses/{course}/members/{user}"))
        .bearer(token)
        .json(&json!({"role": role}))
        .send()
        .await
}

async fn remove_member(http: &Http, token: &str, course: Uuid, user: Uuid) -> Resp {
    http.delete(&format!("/courses/{course}/members/{user}"))
        .bearer(token)
        .send()
        .await
}

async fn publish(http: &Http, token: &str, course: Uuid) -> Resp {
    http.post(&format!("/courses/{course}/publish"))
        .bearer(token)
        .send()
        .await
}

fn ids(page: &Value) -> Vec<Uuid> {
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(id_of)
        .collect()
}

async fn member_role(pool: &PgPool, course: Uuid, user: Uuid) -> Option<String> {
    sqlx::query_scalar!(
        "SELECT role FROM teaching.memberships WHERE course_id = $1 AND user_id = $2",
        course,
        user
    )
    .fetch_optional(pool)
    .await
    .expect("member role")
}

async fn audit_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM events.audit_log"#)
        .fetch_one(pool)
        .await
        .expect("audit count")
}

// ---------------------------------------------------------------------------------------------
// POST /courses
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_teacher_creates_a_draft_course_and_joins_it_as_teacher() {
    db_test(async |pool| {
        let http = setup(&pool).await;
        let teacher = user(&pool, &http, "t@x.edu", Role::Teacher).await;

        let response = http
            .post("/courses")
            .bearer(&teacher.token)
            .json(&json!({"title": "  Algebra 7-A "}))
            .send()
            .await;
        assert_eq!(response.status, 201);
        let course = response.json();
        assert_eq!(course["title"], "Algebra 7-A");
        assert_eq!(course["status"], "draft");
        assert_eq!(course["kind"], "regular");
        assert_eq!(course["owner_id"], teacher.id.to_string());
        assert!(course["created_at"].is_string());

        let members = get(
            &http,
            &teacher.token,
            &format!("/courses/{}/members", course["id"].as_str().unwrap()),
        )
        .await
        .json();
        assert_eq!(
            members["items"],
            json!([{
                "course_id": course["id"],
                "user_id": teacher.id.to_string(),
                "role": "teacher",
                "is_owner": true,
                "source": "direct",
            }])
        );
    })
    .await;
}

#[tokio::test]
async fn creating_a_course_needs_a_teacher_a_title_and_a_login() {
    db_test(async |pool| {
        let http = setup(&pool).await;
        let teacher = user(&pool, &http, "t@x.edu", Role::Teacher).await;
        let student = user(&pool, &http, "s@x.edu", Role::Student).await;

        let denied = http
            .post("/courses")
            .bearer(&student.token)
            .json(&json!({"title": "Mine"}))
            .send()
            .await;
        denied.problem(403);

        for title in ["", "   ", &"a".repeat(201)] {
            let response = http
                .post("/courses")
                .bearer(&teacher.token)
                .json(&json!({"title": title}))
                .send()
                .await;
            let problem = response.problem(422);
            assert_eq!(problem.errors.len(), 1, "{title:?}");
            assert_eq!(problem.errors[0].field, "title", "{title:?}");
        }

        let anonymous = http
            .post("/courses")
            .json(&json!({"title": "Mine"}))
            .send()
            .await;
        anonymous.problem(401);

        let count = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM teaching.courses"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// GET /courses/{id}
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn staff_get_a_course_and_everyone_else_gets_404() {
    db_test(async |pool| {
        let w = world(pool).await;
        let path = format!("/courses/{}", w.course);

        for token in [&w.owner.token, &w.assistant.token] {
            let response = get(&w.http, token, &path).await;
            assert_eq!(response.status, 200);
            assert_eq!(response.json()["id"], w.course.to_string());
        }

        let stranger = user(&w.pool, &w.http, "other@x.edu", Role::Teacher).await;
        get(&w.http, &stranger.token, &path).await.problem(404);
        get(
            &w.http,
            &w.owner.token,
            &format!("/courses/{}", Uuid::now_v7()),
        )
        .await
        .problem(404);
        get(&w.http, &w.owner.token, "/courses/not-a-uuid")
            .await
            .problem(400);
        w.http.get(&path).send().await.problem(401);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Draft and published
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn students_cannot_see_a_draft_course_until_it_is_published() {
    db_test(async |pool| {
        let w = world(pool).await;
        let path = format!("/courses/{}", w.course);
        let members = format!("{path}/members");

        // A student can be enrolled in a draft (done in `world`), but cannot see it.
        get(&w.http, &w.student.token, &path).await.problem(404);
        get(&w.http, &w.student.token, &members).await.problem(404);
        let listed = get(&w.http, &w.student.token, "/me/courses").await.json();
        assert_eq!(ids(&listed), Vec::<Uuid>::new());

        // Staff see the draft everywhere.
        for token in [&w.owner.token, &w.assistant.token] {
            let course = get(&w.http, token, &path).await.json();
            assert_eq!(course["status"], "draft");
            assert_eq!(get(&w.http, token, &members).await.status, 200);
            let listed = get(&w.http, token, "/me/courses").await.json();
            assert_eq!(ids(&listed), [w.course]);
        }

        let published = publish(&w.http, &w.owner.token, w.course).await;
        assert_eq!(published.status, 200);
        assert_eq!(published.json()["status"], "published");

        let course = get(&w.http, &w.student.token, &path).await;
        assert_eq!(course.status, 200);
        assert_eq!(course.json()["status"], "published");
        let roster = get(&w.http, &w.student.token, &members).await;
        assert_eq!(roster.status, 200);
        assert_eq!(roster.json()["items"].as_array().unwrap().len(), 3);
        let listed = get(&w.http, &w.student.token, "/me/courses").await.json();
        assert_eq!(ids(&listed), [w.course]);
    })
    .await;
}

#[tokio::test]
async fn students_cannot_see_an_archived_course_but_staff_can() {
    db_test(async |pool| {
        let w = world(pool).await;
        publish(&w.http, &w.owner.token, w.course).await;
        sqlx::query!(
            "UPDATE teaching.courses SET status = 'archived' WHERE id = $1",
            w.course
        )
        .execute(&w.pool)
        .await
        .unwrap();

        let path = format!("/courses/{}", w.course);
        get(&w.http, &w.student.token, &path).await.problem(404);
        let listed = get(&w.http, &w.student.token, "/me/courses").await.json();
        assert_eq!(ids(&listed), Vec::<Uuid>::new());
        assert_eq!(
            get(&w.http, &w.owner.token, &path).await.json()["status"],
            "archived"
        );
        publish(&w.http, &w.owner.token, w.course)
            .await
            .problem(409);
    })
    .await;
}

#[tokio::test]
async fn publishing_twice_changes_nothing_the_second_time() {
    db_test(async |pool| {
        let w = world(pool).await;
        let updated = || async {
            sqlx::query_scalar!(
                "SELECT updated_at FROM teaching.courses WHERE id = $1",
                w.course
            )
            .fetch_one(&w.pool)
            .await
            .unwrap()
        };
        let before = updated().await;

        let first = publish(&w.http, &w.owner.token, w.course).await;
        assert_eq!(first.status, 200);
        let after_first = updated().await;
        assert!(after_first > before);

        let second = publish(&w.http, &w.owner.token, w.course).await;
        assert_eq!(second.status, 200);
        assert_eq!(second.json(), first.json());
        assert_eq!(updated().await, after_first);
    })
    .await;
}

#[tokio::test]
async fn only_owner_course_teachers_and_admins_publish() {
    db_test(async |pool| {
        let w = world(pool).await;
        let other_teacher = user(&w.pool, &w.http, "t2@x.edu", Role::Teacher).await;
        let stranger = user(&w.pool, &w.http, "stranger@x.edu", Role::Teacher).await;
        put_member(
            &w.http,
            &w.owner.token,
            w.course,
            other_teacher.id,
            "teacher",
        )
        .await;

        publish(&w.http, &w.assistant.token, w.course)
            .await
            .problem(403);
        // A student cannot even see the draft, so the answer is 404 rather than 403.
        publish(&w.http, &w.student.token, w.course)
            .await
            .problem(404);
        publish(&w.http, &stranger.token, w.course)
            .await
            .problem(404);
        publish(&w.http, &w.owner.token, Uuid::now_v7())
            .await
            .problem(404);
        let course = get(&w.http, &w.owner.token, &format!("/courses/{}", w.course)).await;
        assert_eq!(course.json()["status"], "draft");

        assert_eq!(
            publish(&w.http, &other_teacher.token, w.course)
                .await
                .status,
            200
        );

        // Once published, a student gets 403 instead: they can see the course, but not publish it.
        publish(&w.http, &w.student.token, w.course)
            .await
            .problem(403);
        publish(&w.http, &w.assistant.token, w.course)
            .await
            .problem(403);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// GET /me/courses
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn my_courses_lists_only_courses_the_caller_belongs_to() {
    db_test(async |pool| {
        let http = setup(&pool).await;
        let a = user(&pool, &http, "a@x.edu", Role::Teacher).await;
        let b = user(&pool, &http, "b@x.edu", Role::Teacher).await;
        let student = user(&pool, &http, "s@x.edu", Role::Student).await;
        let a1 = create_course(&http, &a.token, "A1").await;
        let a2 = create_course(&http, &a.token, "A2").await;
        let b1 = create_course(&http, &b.token, "B1").await;
        publish(&http, &a.token, a1).await;
        publish(&http, &b.token, b1).await;
        put_member(&http, &a.token, a1, student.id, "student").await;
        put_member(&http, &a.token, a2, student.id, "student").await;
        put_member(&http, &b.token, b1, a.id, "student").await;

        let of_a = get(&http, &a.token, "/me/courses").await.json();
        assert_eq!(ids(&of_a), [a1, a2, b1]);
        let of_b = get(&http, &b.token, "/me/courses").await.json();
        assert_eq!(ids(&of_b), [b1]);
        // The student sees a1 only: a2 is still a draft, b1 is not theirs.
        let of_student = get(&http, &student.token, "/me/courses").await.json();
        assert_eq!(ids(&of_student), [a1]);

        assert_eq!(
            remove_member(&http, &a.token, a1, student.id).await.status,
            204
        );
        let of_student = get(&http, &student.token, "/me/courses").await.json();
        assert_eq!(ids(&of_student), Vec::<Uuid>::new());
    })
    .await;
}

#[tokio::test]
async fn my_courses_pages_with_a_cursor_and_rejects_a_bad_one() {
    db_test(async |pool| {
        let http = setup(&pool).await;
        let teacher = user(&pool, &http, "t@x.edu", Role::Teacher).await;
        let mut created = Vec::new();
        for title in ["One", "Two", "Three"] {
            created.push(create_course(&http, &teacher.token, title).await);
        }

        let first = get(&http, &teacher.token, "/me/courses?limit=2")
            .await
            .json();
        assert_eq!(ids(&first), created[..2]);
        let cursor = first["next_cursor"]
            .as_str()
            .expect("next cursor")
            .to_owned();
        let second = get(
            &http,
            &teacher.token,
            &format!("/me/courses?limit=2&cursor={cursor}"),
        )
        .await
        .json();
        assert_eq!(ids(&second), created[2..]);
        assert!(second["next_cursor"].is_null());

        get(&http, &teacher.token, "/me/courses?cursor=junk")
            .await
            .problem(422);
        get(&http, &teacher.token, "/me/courses?limit=0")
            .await
            .problem(422);
        http.get("/me/courses").send().await.problem(401);
    })
    .await;
}

#[tokio::test]
async fn an_admin_lists_only_courses_they_own() {
    db_test(async |pool| {
        let http = setup(&pool).await;
        let admin = user(&pool, &http, "admin@x.edu", Role::Admin).await;
        let teacher = user(&pool, &http, "t@x.edu", Role::Teacher).await;
        let own = create_course(&http, &admin.token, "Admin's").await;
        create_course(&http, &teacher.token, "Teacher's").await;

        let listed = get(&http, &admin.token, "/me/courses").await.json();
        assert_eq!(ids(&listed), [own]);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// GET /courses/{id}/members
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn members_are_listed_with_ids_only_and_page() {
    db_test(async |pool| {
        let w = world(pool).await;
        let path = format!("/courses/{}/members", w.course);

        let all = get(&w.http, &w.owner.token, &path).await.json();
        let items = all["items"].as_array().unwrap();
        assert_eq!(items.len(), 3);
        for item in items {
            let mut keys: Vec<_> = item.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            assert_eq!(keys, ["course_id", "is_owner", "role", "source", "user_id"]);
        }
        let role_of = |id: Uuid| {
            items
                .iter()
                .find(|m| m["user_id"] == id.to_string())
                .map(|m| {
                    (
                        m["role"].as_str().unwrap(),
                        m["is_owner"].as_bool().unwrap(),
                    )
                })
        };
        assert_eq!(role_of(w.owner.id), Some(("teacher", true)));
        assert_eq!(role_of(w.assistant.id), Some(("assistant", false)));
        assert_eq!(role_of(w.student.id), Some(("student", false)));

        let first = get(&w.http, &w.owner.token, &format!("{path}?limit=2"))
            .await
            .json();
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        let cursor = first["next_cursor"].as_str().expect("next cursor");
        let rest = get(
            &w.http,
            &w.owner.token,
            &format!("{path}?limit=2&cursor={cursor}"),
        )
        .await
        .json();
        assert_eq!(rest["items"].as_array().unwrap().len(), 1);
        assert!(rest["next_cursor"].is_null());
        get(&w.http, &w.owner.token, &format!("{path}?cursor=junk"))
            .await
            .problem(422);

        let stranger = user(&w.pool, &w.http, "other@x.edu", Role::Teacher).await;
        get(&w.http, &stranger.token, &path).await.problem(404);
        get(
            &w.http,
            &w.owner.token,
            &format!("/courses/{}/members", Uuid::now_v7()),
        )
        .await
        .problem(404);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// PUT /courses/{id}/members/{user_id}
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_owner_adds_and_changes_members() {
    db_test(async |pool| {
        let w = world(pool).await;
        let newcomer = user(&w.pool, &w.http, "new@x.edu", Role::Student).await;

        let added = put_member(&w.http, &w.owner.token, w.course, newcomer.id, "student").await;
        assert_eq!(added.status, 200);
        assert_eq!(
            added.json(),
            json!({
                "course_id": w.course.to_string(),
                "user_id": newcomer.id.to_string(),
                "role": "student",
                "is_owner": false,
                "source": "direct",
            })
        );
        let promoted =
            put_member(&w.http, &w.owner.token, w.course, newcomer.id, "assistant").await;
        assert_eq!(promoted.status, 200);
        assert_eq!(promoted.json()["role"], "assistant");
        assert_eq!(
            member_role(&w.pool, w.course, newcomer.id).await.as_deref(),
            Some("assistant")
        );
    })
    .await;
}

#[tokio::test]
async fn putting_the_same_role_again_changes_nothing() {
    db_test(async |pool| {
        let w = world(pool).await;
        let created_at = || async {
            sqlx::query_scalar!(
                "SELECT created_at FROM teaching.memberships WHERE course_id = $1 AND user_id = $2",
                w.course,
                w.student.id
            )
            .fetch_one(&w.pool)
            .await
            .unwrap()
        };
        let before = created_at().await;

        let again = put_member(&w.http, &w.owner.token, w.course, w.student.id, "student").await;
        assert_eq!(again.status, 200);
        assert_eq!(again.json()["role"], "student");
        assert_eq!(created_at().await, before);
        assert_eq!(
            member_role(&w.pool, w.course, w.student.id)
                .await
                .as_deref(),
            Some("student")
        );

        // A role change keeps the original `created_at` too.
        put_member(&w.http, &w.owner.token, w.course, w.student.id, "assistant").await;
        assert_eq!(created_at().await, before);
    })
    .await;
}

#[tokio::test]
async fn students_and_strangers_cannot_manage_members() {
    db_test(async |pool| {
        let w = world(pool).await;
        let newcomer = user(&w.pool, &w.http, "new@x.edu", Role::Student).await;
        let stranger = user(&w.pool, &w.http, "other@x.edu", Role::Teacher).await;
        publish(&w.http, &w.owner.token, w.course).await;

        for role in ["student", "assistant", "teacher"] {
            // Even a no-op is refused: a student may not manage anyone.
            let response = put_member(&w.http, &w.student.token, w.course, newcomer.id, role).await;
            response.problem(403);
            put_member(&w.http, &w.student.token, w.course, w.student.id, role)
                .await
                .problem(403);
            put_member(&w.http, &stranger.token, w.course, newcomer.id, role)
                .await
                .problem(404);
        }
        assert_eq!(member_role(&w.pool, w.course, newcomer.id).await, None);
        assert_eq!(
            member_role(&w.pool, w.course, w.student.id)
                .await
                .as_deref(),
            Some("student")
        );

        put_member(
            &w.http,
            &w.owner.token,
            Uuid::now_v7(),
            newcomer.id,
            "student",
        )
        .await
        .problem(404);
        remove_member(&w.http, &w.student.token, w.course, w.assistant.id)
            .await
            .problem(403);
        assert_eq!(
            member_role(&w.pool, w.course, w.assistant.id)
                .await
                .as_deref(),
            Some("assistant")
        );
    })
    .await;
}

#[tokio::test]
async fn an_assistant_manages_students_but_not_staff() {
    db_test(async |pool| {
        let w = world(pool).await;
        let newcomer = user(&w.pool, &w.http, "new@x.edu", Role::Student).await;
        let teacher = user(&w.pool, &w.http, "t2@x.edu", Role::Teacher).await;
        put_member(&w.http, &w.owner.token, w.course, teacher.id, "teacher").await;

        // May add students...
        let added = put_member(
            &w.http,
            &w.assistant.token,
            w.course,
            newcomer.id,
            "student",
        )
        .await;
        assert_eq!(added.status, 200);
        // ...but not promote one to teacher or assistant,
        for role in ["teacher", "assistant"] {
            put_member(&w.http, &w.assistant.token, w.course, w.student.id, role)
                .await
                .problem(403);
        }
        assert_eq!(
            member_role(&w.pool, w.course, w.student.id)
                .await
                .as_deref(),
            Some("student")
        );
        // nor add staff,
        let third = user(&w.pool, &w.http, "t3@x.edu", Role::Teacher).await;
        put_member(&w.http, &w.assistant.token, w.course, third.id, "teacher")
            .await
            .problem(403);
        assert_eq!(member_role(&w.pool, w.course, third.id).await, None);
        // nor change a teacher's role or remove a teacher,
        put_member(&w.http, &w.assistant.token, w.course, teacher.id, "student")
            .await
            .problem(403);
        remove_member(&w.http, &w.assistant.token, w.course, teacher.id)
            .await
            .problem(403);
        // and not even remove the owner (the owner check never gets that far).
        remove_member(&w.http, &w.assistant.token, w.course, w.owner.id)
            .await
            .problem(403);
        assert_eq!(
            member_role(&w.pool, w.course, teacher.id).await.as_deref(),
            Some("teacher")
        );

        // It may remove a student.
        assert_eq!(
            remove_member(&w.http, &w.assistant.token, w.course, newcomer.id)
                .await
                .status,
            204
        );
    })
    .await;
}

#[tokio::test]
async fn a_course_teacher_manages_staff_and_students() {
    db_test(async |pool| {
        let w = world(pool).await;
        let teacher = user(&w.pool, &w.http, "t2@x.edu", Role::Teacher).await;
        let helper = user(&w.pool, &w.http, "h@x.edu", Role::Student).await;
        let colleague = user(&w.pool, &w.http, "t3@x.edu", Role::Teacher).await;
        put_member(&w.http, &w.owner.token, w.course, teacher.id, "teacher").await;

        for (who, role) in [(helper.id, "assistant"), (colleague.id, "teacher")] {
            let added = put_member(&w.http, &teacher.token, w.course, who, role).await;
            assert_eq!(added.status, 200, "{role}");
            assert_eq!(
                member_role(&w.pool, w.course, who).await.as_deref(),
                Some(role)
            );
        }
        for who in [helper.id, colleague.id] {
            assert_eq!(
                remove_member(&w.http, &teacher.token, w.course, who)
                    .await
                    .status,
                204
            );
            assert_eq!(member_role(&w.pool, w.course, who).await, None);
        }
    })
    .await;
}

#[tokio::test]
async fn members_must_exist_and_may_hold_only_roles_their_institution_role_allows() {
    db_test(async |pool| {
        let w = world(pool).await;
        let institution_student = user(&w.pool, &w.http, "s2@x.edu", Role::Student).await;
        let admin = user(&w.pool, &w.http, "admin@x.edu", Role::Admin).await;
        let teacher = user(&w.pool, &w.http, "t2@x.edu", Role::Teacher).await;

        let unknown =
            put_member(&w.http, &w.owner.token, w.course, Uuid::now_v7(), "student").await;
        assert_eq!(unknown.problem(422).errors[0].field, "user_id");

        let too_junior = put_member(
            &w.http,
            &w.owner.token,
            w.course,
            institution_student.id,
            "teacher",
        )
        .await;
        assert_eq!(too_junior.problem(422).errors[0].field, "role");
        assert_eq!(
            member_role(&w.pool, w.course, institution_student.id).await,
            None
        );

        let admin_member = put_member(&w.http, &w.owner.token, w.course, admin.id, "teacher").await;
        assert_eq!(admin_member.problem(422).errors[0].field, "role");

        // Institution students may be assistants and students; teachers may be anything.
        for (who, role) in [
            (institution_student.id, "assistant"),
            (teacher.id, "student"),
            (teacher.id, "assistant"),
            (teacher.id, "teacher"),
        ] {
            assert_eq!(
                put_member(&w.http, &w.owner.token, w.course, who, role)
                    .await
                    .status,
                200,
                "{role}"
            );
        }

        let bad_role = w
            .http
            .put(&format!("/courses/{}/members/{}", w.course, teacher.id))
            .bearer(&w.owner.token)
            .json(&json!({"role": "owner"}))
            .send()
            .await;
        bad_role.problem(422);
    })
    .await;
}

async fn disable(pool: &PgPool, user: Uuid) {
    sqlx::query!(
        "UPDATE identity.users SET status = 'disabled' WHERE id = $1",
        user
    )
    .execute(pool)
    .await
    .expect("disable user");
}

#[tokio::test]
async fn disabled_users_cannot_be_added_as_members() {
    db_test(async |pool| {
        let w = world(pool).await;
        let admin = user(&w.pool, &w.http, "admin@x.edu", Role::Admin).await;
        let gone = user(&w.pool, &w.http, "gone@x.edu", Role::Student).await;
        disable(&w.pool, gone.id).await;

        // A course teacher is refused with an error on the user.
        let refused = put_member(&w.http, &w.owner.token, w.course, gone.id, "student").await;
        let problem = refused.problem(422);
        assert_eq!(problem.errors.len(), 1);
        assert_eq!(problem.errors[0].field, "user_id");
        assert_eq!(member_role(&w.pool, w.course, gone.id).await, None);

        // So is an admin, and a refused change leaves no audit row behind.
        let admins = create_course(&w.http, &admin.token, "Admin's course").await;
        let audited = audit_count(&w.pool).await;
        let refused = put_member(&w.http, &admin.token, admins, gone.id, "student").await;
        assert_eq!(refused.problem(422).errors[0].field, "user_id");
        assert_eq!(member_role(&w.pool, admins, gone.id).await, None);
        assert_eq!(audit_count(&w.pool).await, audited);
    })
    .await;
}

#[tokio::test]
async fn putting_the_same_role_for_a_member_who_was_disabled_later_is_still_a_no_op() {
    db_test(async |pool| {
        let w = world(pool).await;
        disable(&w.pool, w.student.id).await;

        let again = put_member(&w.http, &w.owner.token, w.course, w.student.id, "student").await;
        assert_eq!(again.status, 200);
        assert_eq!(
            member_role(&w.pool, w.course, w.student.id)
                .await
                .as_deref(),
            Some("student")
        );
    })
    .await;
}

#[tokio::test]
async fn the_owner_must_stay_a_teacher_and_cannot_be_removed() {
    db_test(async |pool| {
        let w = world(pool).await;
        let teacher = user(&w.pool, &w.http, "t2@x.edu", Role::Teacher).await;
        put_member(&w.http, &w.owner.token, w.course, teacher.id, "teacher").await;

        for role in ["assistant", "student"] {
            put_member(&w.http, &w.owner.token, w.course, w.owner.id, role)
                .await
                .problem(409);
            put_member(&w.http, &teacher.token, w.course, w.owner.id, role)
                .await
                .problem(409);
        }
        remove_member(&w.http, &w.owner.token, w.course, w.owner.id)
            .await
            .problem(409);
        remove_member(&w.http, &teacher.token, w.course, w.owner.id)
            .await
            .problem(409);
        assert_eq!(
            member_role(&w.pool, w.course, w.owner.id).await.as_deref(),
            Some("teacher")
        );
        // Putting the role they have is fine.
        assert_eq!(
            put_member(&w.http, &w.owner.token, w.course, w.owner.id, "teacher")
                .await
                .json()["is_owner"],
            true
        );
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// DELETE /courses/{id}/members/{user_id}
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn removing_a_member_takes_their_access_away() {
    db_test(async |pool| {
        let w = world(pool).await;
        publish(&w.http, &w.owner.token, w.course).await;
        let path = format!("/courses/{}", w.course);
        assert_eq!(get(&w.http, &w.student.token, &path).await.status, 200);

        let removed = remove_member(&w.http, &w.owner.token, w.course, w.student.id).await;
        assert_eq!(removed.status, 204);
        assert!(removed.body.is_empty());
        get(&w.http, &w.student.token, &path).await.problem(404);

        // Removing someone who is not a member is a 404, and so is an unknown course.
        remove_member(&w.http, &w.owner.token, w.course, w.student.id)
            .await
            .problem(404);
        remove_member(&w.http, &w.owner.token, Uuid::now_v7(), w.student.id)
            .await
            .problem(404);
        let stranger = user(&w.pool, &w.http, "other@x.edu", Role::Teacher).await;
        remove_member(&w.http, &stranger.token, w.course, w.assistant.id)
            .await
            .problem(404);
        w.http
            .delete(&format!("{path}/members/{}", w.assistant.id))
            .send()
            .await
            .problem(401);
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// Admins
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn an_admin_acts_as_an_implicit_owner_without_a_membership() {
    db_test(async |pool| {
        let w = world(pool).await;
        let admin = user(&w.pool, &w.http, "admin@x.edu", Role::Admin).await;
        let newcomer = user(&w.pool, &w.http, "new@x.edu", Role::Teacher).await;
        let path = format!("/courses/{}", w.course);

        assert_eq!(
            get(&w.http, &admin.token, &path).await.json()["status"],
            "draft"
        );
        let members = get(&w.http, &admin.token, &format!("{path}/members")).await;
        assert_eq!(members.json()["items"].as_array().unwrap().len(), 3);

        assert_eq!(
            put_member(&w.http, &admin.token, w.course, newcomer.id, "teacher")
                .await
                .status,
            200
        );
        assert_eq!(
            put_member(&w.http, &admin.token, w.course, w.student.id, "assistant")
                .await
                .status,
            200
        );
        assert_eq!(
            remove_member(&w.http, &admin.token, w.course, newcomer.id)
                .await
                .status,
            204
        );
        assert_eq!(publish(&w.http, &admin.token, w.course).await.status, 200);
        assert_eq!(member_role(&w.pool, w.course, admin.id).await, None);
    })
    .await;
}

#[tokio::test]
async fn an_admin_course_has_no_owner_membership_and_its_owner_cannot_be_added_as_a_member() {
    db_test(async |pool| {
        let http = setup(&pool).await;
        let admin = user(&pool, &http, "admin@x.edu", Role::Admin).await;
        let course = create_course(&http, &admin.token, "Admin's course").await;

        let members = get(&http, &admin.token, &format!("/courses/{course}/members")).await;
        assert_eq!(members.json()["items"], json!([]));
        assert_eq!(member_role(&pool, course, admin.id).await, None);
        put_member(&http, &admin.token, course, admin.id, "teacher")
            .await
            .problem(422);
    })
    .await;
}

#[tokio::test]
async fn admin_actions_are_audited_and_other_actions_are_not() {
    db_test(async |pool| {
        let http = setup(&pool).await;
        let admin = user(&pool, &http, "admin@x.edu", Role::Admin).await;
        let teacher = user(&pool, &http, "t@x.edu", Role::Teacher).await;
        let student = user(&pool, &http, "s@x.edu", Role::Student).await;

        // Teacher actions write nothing.
        let own = create_course(&http, &teacher.token, "Teacher's").await;
        put_member(&http, &teacher.token, own, student.id, "student").await;
        publish(&http, &teacher.token, own).await;
        remove_member(&http, &teacher.token, own, student.id).await;
        assert_eq!(audit_count(&pool).await, 0);

        let course = create_course(&http, &admin.token, "Admin's").await;
        put_member(&http, &admin.token, course, student.id, "student").await;
        // A repeat with the same role changes nothing, so it is not audited again.
        put_member(&http, &admin.token, course, student.id, "student").await;
        publish(&http, &admin.token, course).await;
        publish(&http, &admin.token, course).await;
        remove_member(&http, &admin.token, course, student.id).await;
        // The admin also manages a course they do not own.
        put_member(&http, &admin.token, own, student.id, "student").await;

        let rows = sqlx::query!(
            "SELECT actor_id, action, resource_type, resource_id, meta
             FROM events.audit_log ORDER BY at, id"
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let summary: Vec<_> = rows
            .iter()
            .map(|r| (r.action.as_str(), r.resource_id))
            .collect();
        assert_eq!(
            summary,
            [
                ("course.create", course),
                ("course.member.put", course),
                ("course.publish", course),
                ("course.member.remove", course),
                ("course.member.put", own),
            ]
        );
        for row in &rows {
            assert_eq!(row.actor_id, Some(admin.id));
            assert_eq!(row.resource_type, "course");
        }
        assert_eq!(
            rows[1].meta,
            json!({"user_id": student.id.to_string(), "role": "student"})
        );
        assert_eq!(rows[0].meta, json!({}));
    })
    .await;
}

// ---------------------------------------------------------------------------------------------
// OpenAPI
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn every_course_endpoint_is_in_the_openapi_document() {
    db_test(async |pool| {
        let base = common::spawn(pool).await;
        let docs = reqwest::get(format!("{base}/api/v1/docs"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        for path in ["/api/v1/courses/{id}/publish", "/api/v1/me/courses"] {
            assert!(docs.contains(path), "{path} missing from the docs page");
        }
        let response = reqwest::get(format!("{base}/api/v1/openapi.json"))
            .await
            .unwrap();
        let doc: Value = serde_json::from_slice(&response.bytes().await.unwrap()).unwrap();
        for (path, method) in [
            ("/api/v1/courses", "post"),
            ("/api/v1/courses/{id}", "get"),
            ("/api/v1/courses/{id}/publish", "post"),
            ("/api/v1/me/courses", "get"),
            ("/api/v1/courses/{id}/members", "get"),
            ("/api/v1/courses/{id}/members/{user_id}", "put"),
            ("/api/v1/courses/{id}/members/{user_id}", "delete"),
        ] {
            let operation = &doc["paths"][path][method];
            assert!(operation.is_object(), "{method} {path} missing");
            assert_eq!(operation["tags"], json!(["courses"]), "{method} {path}");
            assert!(
                operation["security"].is_array(),
                "{method} {path} has no security"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_course_is_created_in_an_existing_term_and_an_unknown_term_is_a_422() {
    db_test(async |pool| {
        let http = setup(&pool).await;
        let teacher = user(&pool, &http, "t@x.edu", Role::Teacher).await;
        let term = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO org.terms (id, name, starts_on, ends_on)
             VALUES ($1, 'Fall', '2026-09-01', '2027-01-31')",
            term
        )
        .execute(&pool)
        .await
        .unwrap();
        let courses = || async {
            sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM teaching.courses"#)
                .fetch_one(&pool)
                .await
                .unwrap()
        };

        let unknown = http
            .post("/courses")
            .bearer(&teacher.token)
            .json(&json!({"title": "Algebra", "term_id": Uuid::now_v7()}))
            .send()
            .await;
        let problem = unknown.problem(422);
        assert_eq!(problem.errors.len(), 1);
        assert_eq!(problem.errors[0].field, "term_id");
        assert_eq!(
            courses().await,
            0,
            "nothing was created, and nothing is a 500"
        );

        let created = http
            .post("/courses")
            .bearer(&teacher.token)
            .json(&json!({"title": "Algebra", "term_id": term}))
            .send()
            .await;
        assert_eq!(created.status, 201);
        assert_eq!(created.json()["term_id"], term.to_string());
        let id = created.json()["id"].as_str().unwrap().to_owned();
        let shown = get(&http, &teacher.token, &format!("/courses/{id}"))
            .await
            .json();
        assert_eq!(shown["term_id"], term.to_string());

        let without = http
            .post("/courses")
            .bearer(&teacher.token)
            .json(&json!({"title": "Geometry"}))
            .send()
            .await;
        assert_eq!(without.status, 201);
        assert_eq!(without.json()["term_id"], Value::Null);
        let null = http
            .post("/courses")
            .bearer(&teacher.token)
            .json(&json!({"title": "Calculus", "term_id": null}))
            .send()
            .await;
        assert_eq!(null.json()["term_id"], Value::Null);
    })
    .await;
}
