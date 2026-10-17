//! Who is on a course and who may do what in it as accounts change (ADR-0037): the active
//! student roster, former students, role changes, ownership transfer, the bulk group writes of a
//! rollover. Over HTTP, with a database each.

mod common;

use common::{Env, db_test, essay, id_of};
use serde_json::{Value, json};
use server::identity::Role;
use sqlx::PgPool;
use uuid::Uuid;

/// The `meta` of every audit row with this action, oldest first.
async fn audit_meta(pool: &PgPool, action: &str) -> Vec<Value> {
    sqlx::query_scalar!(
        "SELECT meta FROM events.audit_log WHERE action = $1 ORDER BY id",
        action
    )
    .fetch_all(pool)
    .await
    .expect("audit rows")
}

async fn set_role(env: &Env, admin: &str, user: Uuid, role: &str) -> common::Resp {
    env.call(
        "PATCH",
        admin,
        &format!("/admin/users/{user}"),
        Some(json!({"role": role})),
    )
    .await
}

async fn gradebook(env: &Env, token: &str, course: Uuid) -> Value {
    env.ok(
        "GET",
        token,
        &format!("/courses/{course}/gradebook?limit=100"),
        None,
        200,
    )
    .await
}

/// `(user, former)` for every row of a gradebook page.
fn rows(book: &Value) -> Vec<(String, bool)> {
    book["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|row| {
            (
                row["user_id"].as_str().expect("user id").to_owned(),
                row["former"].as_bool().expect("former"),
            )
        })
        .collect()
}

async fn grade(
    env: &Env,
    token: &str,
    assignment: Uuid,
    student: Uuid,
    score: &str,
) -> common::Resp {
    env.call(
        "PUT",
        token,
        &format!("/assignments/{assignment}/grades/{student}"),
        Some(json!({"score": score, "feedback": null})),
    )
    .await
}

async fn publish_all(env: &Env, token: &str, assignment: Uuid) -> Value {
    env.ok(
        "POST",
        token,
        &format!("/assignments/{assignment}/grades/publish"),
        Some(json!({"students": "all"})),
        200,
    )
    .await
}

async fn members(env: &Env, token: &str, course: Uuid) -> Vec<String> {
    env.ok(
        "GET",
        token,
        &format!("/courses/{course}/members?limit=100"),
        None,
        200,
    )
    .await["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|m| m["user_id"].as_str().expect("user id").to_owned())
        .collect()
}

// --- role changes (A-01) ---------------------------------------------------------------------

#[tokio::test]
async fn demoting_a_teacher_removes_their_teacher_memberships_and_audits_each() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let co = env.user("co@x.edu", Role::Teacher).await;
        let (first, _) = env.course(&owner, &[]).await;
        let (second, _) = env.course(&owner, &[]).await;
        for course in [first, second] {
            let path = format!("/courses/{course}/members/{}", co.id);
            env.ok(
                "PUT",
                &owner.token,
                &path,
                Some(json!({"role": "teacher"})),
                200,
            )
            .await;
        }
        env.ok("GET", &co.token, &format!("/courses/{first}"), None, 200)
            .await;

        let demoted = set_role(&env, &admin.token, co.id, "student").await;
        assert_eq!(
            demoted.status,
            200,
            "{}",
            String::from_utf8_lossy(&demoted.body)
        );

        let left = sqlx::query_scalar!(
            "SELECT count(*) AS \"n!\" FROM teaching.memberships WHERE user_id = $1",
            co.id
        )
        .fetch_one(&env.pool)
        .await
        .unwrap();
        assert_eq!(left, 0, "both teacher memberships are gone");
        let audited = audit_meta(&env.pool, "course.member.remove").await;
        assert_eq!(audited.len(), 2);
        for meta in &audited {
            assert_eq!(meta["user_id"], json!(co.id));
            assert_eq!(meta["role"], "teacher");
            assert_eq!(meta["reason"], "role_change");
        }
        // Nothing of their old powers is left, and the course is not even visible to them.
        for course in [first, second] {
            let path = format!("/courses/{course}/modules");
            let r = env
                .call("POST", &co.token, &path, Some(json!({"title": "x"})))
                .await;
            assert_eq!(r.status, 404);
        }
        assert_eq!(audit_meta(&env.pool, "user.role_change").await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_demotion_is_refused_while_the_user_owns_a_course() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let heir = env.user("heir@x.edu", Role::Teacher).await;
        let (course, _) = env.course(&owner, &[]).await;

        let refused = set_role(&env, &admin.token, owner.id, "student").await;
        assert_eq!(refused.problem(409).status, 409);
        let me = env.ok("GET", &owner.token, "/me", None, 200).await;
        assert_eq!(me["role"], "teacher", "the refusal rolled the change back");
        env.ok(
            "GET",
            &owner.token,
            &format!("/courses/{course}"),
            None,
            200,
        )
        .await;
        assert!(audit_meta(&env.pool, "user.role_change").await.is_empty());

        env.ok(
            "PUT",
            &admin.token,
            &format!("/courses/{course}/owner"),
            Some(json!({"user_id": heir.id})),
            200,
        )
        .await;
        let allowed = set_role(&env, &admin.token, owner.id, "student").await;
        assert_eq!(allowed.status, 200);
        // The former owner was a course teacher of the course; that went with the demotion.
        let left = sqlx::query_scalar!(
            "SELECT count(*) AS \"n!\" FROM teaching.memberships WHERE user_id = $1",
            owner.id
        )
        .fetch_one(&env.pool)
        .await
        .unwrap();
        assert_eq!(left, 0);
    })
    .await;
}

#[tokio::test]
async fn demoting_an_admin_who_owns_a_course_is_refused_too() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let other = env.user("other-admin@x.edu", Role::Admin).await;
        env.ok(
            "POST",
            &other.token,
            "/courses",
            Some(json!({"title": "Admin's"})),
            201,
        )
        .await;
        let refused = set_role(&env, &admin.token, other.id, "student").await;
        refused.problem(409);
        // To teacher is not a demotion away from teaching.
        assert_eq!(
            set_role(&env, &admin.token, other.id, "teacher")
                .await
                .status,
            200
        );
    })
    .await;
}

#[tokio::test]
async fn an_owner_who_is_an_institution_student_has_no_powers_even_if_the_row_says_so() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let (course, module) = env.course(&owner, &[]).await;
        // A state the rules prevent, forced in the database: policy must still refuse it.
        sqlx::query!(
            "UPDATE identity.users SET role = 'student' WHERE id = $1",
            owner.id
        )
        .execute(&env.pool)
        .await
        .unwrap();
        for (method, path, body) in [
            ("GET", format!("/courses/{course}"), None),
            (
                "POST",
                format!("/courses/{course}/modules"),
                Some(json!({"title": "x"})),
            ),
            (
                "POST",
                format!("/modules/{module}/items"),
                Some(json!({"kind": "text", "title": "t"})),
            ),
            ("POST", format!("/courses/{course}/publish"), None),
        ] {
            let r = env.call(method, &owner.token, &path, body).await;
            assert!(
                r.status == 404 || r.status == 403,
                "{method} {path} -> {}",
                r.status
            );
        }
    })
    .await;
}

// --- creating a course and adding a member are checked against the account (race) ----------

#[tokio::test]
async fn a_user_who_is_no_longer_a_teacher_cannot_create_a_course() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let teacher = env.user("teacher@x.edu", Role::Teacher).await;
        env.ok("GET", &teacher.token, "/me", None, 200).await;
        // The session role is read per request; the service reads the account again under a lock.
        set_role(&env, &admin.token, teacher.id, "student").await;
        let r = env
            .call(
                "POST",
                &teacher.token,
                "/courses",
                Some(json!({"title": "late"})),
            )
            .await;
        assert_eq!(r.status, 403);
    })
    .await;
}

// --- ownership transfer (A-01) ---------------------------------------------------------------

#[tokio::test]
async fn the_owner_hands_a_course_to_another_teacher() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let heir = env.user("heir@x.edu", Role::Teacher).await;
        let (course, _) = env.course(&owner, &[]).await;

        let handed = env
            .ok(
                "PUT",
                &owner.token,
                &format!("/courses/{course}/owner"),
                Some(json!({"user_id": heir.id})),
                200,
            )
            .await;
        assert_eq!(handed["owner_id"], json!(heir.id));
        let roster = env
            .ok(
                "GET",
                &heir.token,
                &format!("/courses/{course}/members"),
                None,
                200,
            )
            .await;
        let heir_entry = roster["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["user_id"] == json!(heir.id))
            .expect("the new owner is on the roster");
        assert_eq!(heir_entry["role"], "teacher");
        assert_eq!(heir_entry["is_owner"], true);
        // The previous owner stays a teacher but cannot hand the course over any more.
        env.ok(
            "GET",
            &owner.token,
            &format!("/courses/{course}"),
            None,
            200,
        )
        .await;
        let again = env
            .call(
                "PUT",
                &owner.token,
                &format!("/courses/{course}/owner"),
                Some(json!({"user_id": owner.id})),
            )
            .await;
        assert_eq!(again.status, 403);
        let audited = audit_meta(&env.pool, "course.owner.transfer").await;
        assert_eq!(audited, [json!({"from": owner.id, "to": heir.id})]);
        // Idempotent for the current owner, and nothing more is audited.
        env.ok(
            "PUT",
            &heir.token,
            &format!("/courses/{course}/owner"),
            Some(json!({"user_id": heir.id})),
            200,
        )
        .await;
        assert_eq!(
            audit_meta(&env.pool, "course.owner.transfer").await.len(),
            1
        );
    })
    .await;
}

#[tokio::test]
async fn an_admin_may_take_a_course_and_only_a_teacher_or_admin_may_own_one() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let student = env.user("student@x.edu", Role::Student).await;
        let sleeper = env.user("sleeper@x.edu", Role::Teacher).await;
        let (course, _) = env.course(&owner, &[&student]).await;
        let path = format!("/courses/{course}/owner");

        for (who, label) in [
            (student.id, "a student"),
            (sleeper.id, "a disabled teacher"),
        ] {
            if label == "a disabled teacher" {
                sqlx::query!(
                    "UPDATE identity.users SET status = 'disabled' WHERE id = $1",
                    sleeper.id
                )
                .execute(&env.pool)
                .await
                .unwrap();
            }
            let r = env
                .call("PUT", &owner.token, &path, Some(json!({"user_id": who})))
                .await;
            assert_eq!(r.problem(422).errors[0].field, "user_id", "{label}");
        }
        let unknown = env
            .call(
                "PUT",
                &owner.token,
                &path,
                Some(json!({"user_id": Uuid::now_v7()})),
            )
            .await;
        assert_eq!(unknown.problem(422).errors[0].field, "user_id");

        // An admin owns without a membership.
        let taken = env
            .ok(
                "PUT",
                &admin.token,
                &path,
                Some(json!({"user_id": admin.id})),
                200,
            )
            .await;
        assert_eq!(taken["owner_id"], json!(admin.id));
        let admin_membership = sqlx::query_scalar!(
            "SELECT count(*) AS \"n!\" FROM teaching.memberships WHERE user_id = $1",
            admin.id
        )
        .fetch_one(&env.pool)
        .await
        .unwrap();
        assert_eq!(admin_membership, 0);
        // The former owner is still a course teacher and can read the course.
        env.ok(
            "GET",
            &owner.token,
            &format!("/courses/{course}"),
            None,
            200,
        )
        .await;
    })
    .await;
}

#[tokio::test]
async fn only_the_owner_or_an_admin_may_hand_a_course_over() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let co = env.user("co@x.edu", Role::Teacher).await;
        let assistant = env.user("assistant@x.edu", Role::Student).await;
        let outsider = env.user("outsider@x.edu", Role::Teacher).await;
        let (course, _) = env.course(&owner, &[]).await;
        env.ok(
            "PUT",
            &owner.token,
            &format!("/courses/{course}/members/{}", co.id),
            Some(json!({"role": "teacher"})),
            200,
        )
        .await;
        env.ok(
            "PUT",
            &owner.token,
            &format!("/courses/{course}/members/{}", assistant.id),
            Some(json!({"role": "assistant"})),
            200,
        )
        .await;
        let path = format!("/courses/{course}/owner");
        let body = || Some(json!({"user_id": outsider.id}));
        assert_eq!(env.call("PUT", &co.token, &path, body()).await.status, 403);
        assert_eq!(
            env.call("PUT", &assistant.token, &path, body())
                .await
                .status,
            403
        );
        assert_eq!(
            env.call("PUT", &outsider.token, &path, body()).await.status,
            404
        );
    })
    .await;
}

// --- the roster of a course (A-02, disabled students, former students) -----------------------

struct Linked {
    env: Env,
    admin: common::Account,
    owner: common::Account,
    student: common::Account,
    group: Uuid,
    course: Uuid,
    assignment: Uuid,
}

/// A published course with a group of one graded and published student linked to it.
async fn linked(pool: PgPool) -> Linked {
    let env = Env::new(pool).await;
    let admin = env.user("admin@x.edu", Role::Admin).await;
    let owner = env.user("owner@x.edu", Role::Teacher).await;
    let student = env.user("s@x.edu", Role::Student).await;
    let (course, module) = env.course(&owner, &[]).await;
    let assignment = env.assignment(&owner, module, "10").await;
    let group = id_of(
        &env.ok(
            "POST",
            &admin.token,
            "/groups",
            Some(json!({"name": "7-A"})),
            201,
        )
        .await,
    );
    env.ok(
        "POST",
        &admin.token,
        &format!("/groups/{group}/members"),
        Some(json!({"user_ids": [student.id]})),
        200,
    )
    .await;
    env.ok(
        "POST",
        &owner.token,
        &format!("/courses/{course}/groups"),
        Some(json!({"group_id": group})),
        201,
    )
    .await;
    assert_eq!(
        grade(&env, &owner.token, assignment, student.id, "7")
            .await
            .status,
        200
    );
    publish_all(&env, &owner.token, assignment).await;
    // An unpublished change on top of the published grade.
    assert_eq!(
        grade(&env, &owner.token, assignment, student.id, "5")
            .await
            .status,
        200
    );
    Linked {
        env,
        admin,
        owner,
        student,
        group,
        course,
        assignment,
    }
}

#[tokio::test]
async fn a_group_student_who_is_promoted_leaves_the_roster_and_becomes_a_former_student() {
    db_test(async |pool| {
        let l = linked(pool).await;
        let env = &l.env;
        assert!(
            members(env, &l.owner.token, l.course)
                .await
                .contains(&l.student.id.to_string())
        );

        assert_eq!(
            set_role(env, &l.admin.token, l.student.id, "teacher")
                .await
                .status,
            200
        );

        assert!(
            !members(env, &l.owner.token, l.course)
                .await
                .contains(&l.student.id.to_string()),
            "a teacher is not a student of the course"
        );
        let book = gradebook(env, &l.owner.token, l.course).await;
        assert_eq!(rows(&book), [(l.student.id.to_string(), true)]);
        assert_eq!(
            book["items"][0]["cells"][0]["draft_score"], "5.00",
            "data intact"
        );
        assert_eq!(book["items"][0]["cells"][0]["published_score"], "7.00");
        // Read-only: no grade write, and "publish all" leaves the published value alone.
        let write = grade(env, &l.owner.token, l.assignment, l.student.id, "9").await;
        assert_eq!(write.problem(409).status, 409);
        let published = publish_all(env, &l.owner.token, l.assignment).await;
        assert_eq!(published["published"], json!([]));
        let book = gradebook(env, &l.owner.token, l.course).await;
        assert_eq!(book["items"][0]["cells"][0]["published_score"], "7.00");
        // A listed publish names them.
        let listed = env
            .call(
                "POST",
                &l.owner.token,
                &format!("/assignments/{}/grades/publish", l.assignment),
                Some(json!({"students": [l.student.id]})),
            )
            .await;
        assert_eq!(listed.problem(422).errors[0].field, "students[0]");
        // Unknown users stay a 404.
        let unknown = grade(env, &l.owner.token, l.assignment, Uuid::now_v7(), "1").await;
        assert_eq!(unknown.status, 404);
    })
    .await;
}

#[tokio::test]
async fn a_former_student_reads_their_published_grades_and_nothing_else() {
    db_test(async |pool| {
        let l = linked(pool).await;
        let env = &l.env;
        let course = l.course;
        // Students see the course while they are on it.
        env.ok(
            "GET",
            &l.student.token,
            &format!("/courses/{course}"),
            None,
            200,
        )
        .await;

        env.ok(
            "POST",
            &l.admin.token,
            &format!("/groups/{}/members/remove", l.group),
            Some(json!({"user_ids": [l.student.id]})),
            200,
        )
        .await;

        let mine = env
            .ok(
                "GET",
                &l.student.token,
                &format!("/courses/{course}/my-grades"),
                None,
                200,
            )
            .await;
        let grades = mine["items"].as_array().expect("items");
        assert_eq!(grades.len(), 1);
        assert_eq!(
            grades[0]["score"], "7.00",
            "the published value, never the draft"
        );
        assert!(!mine.to_string().contains("draft"));
        for path in [
            format!("/courses/{course}"),
            format!("/courses/{course}/content"),
            format!("/courses/{course}/members"),
            format!("/items/{}", l.assignment),
            format!("/assignments/{}/submission", l.assignment),
        ] {
            let r = env.call("GET", &l.student.token, &path, None).await;
            assert_eq!(r.status, 404, "{path}");
        }
        // Someone with no published grade in the course gets the usual 404.
        let stranger = env.user("stranger@x.edu", Role::Student).await;
        let r = env
            .call(
                "GET",
                &stranger.token,
                &format!("/courses/{course}/my-grades"),
                None,
            )
            .await;
        assert_eq!(r.status, 404);
    })
    .await;
}

#[tokio::test]
async fn a_disabled_student_is_off_the_roster_but_their_grades_are_kept_and_read_only() {
    db_test(async |pool| {
        let l = linked(pool).await;
        let env = &l.env;
        let other = env.user("other@x.edu", Role::Student).await;
        env.ok(
            "POST",
            &l.admin.token,
            &format!("/groups/{}/members", l.group),
            Some(json!({"user_ids": [other.id]})),
            200,
        )
        .await;

        env.ok(
            "PATCH",
            &l.admin.token,
            &format!("/admin/users/{}", l.student.id),
            Some(json!({"status": "disabled"})),
            200,
        )
        .await;

        let roster = members(env, &l.owner.token, l.course).await;
        assert!(!roster.contains(&l.student.id.to_string()));
        assert!(roster.contains(&other.id.to_string()));
        let book = gradebook(env, &l.owner.token, l.course).await;
        let mut seen = rows(&book);
        seen.sort();
        let mut expected = vec![
            (l.student.id.to_string(), true),
            (other.id.to_string(), false),
        ];
        expected.sort();
        assert_eq!(seen, expected);
        assert_eq!(
            grade(env, &l.owner.token, l.assignment, l.student.id, "9")
                .await
                .status,
            409
        );
        // Another student's grade can still be written and published; the disabled one is skipped.
        assert_eq!(
            grade(env, &l.owner.token, l.assignment, other.id, "3")
                .await
                .status,
            200
        );
        let published = publish_all(env, &l.owner.token, l.assignment).await;
        assert_eq!(published["published"], json!([other.id]));

        // Enabled again, they are back with everything they had.
        env.ok(
            "PATCH",
            &l.admin.token,
            &format!("/admin/users/{}", l.student.id),
            Some(json!({"status": "active"})),
            200,
        )
        .await;
        assert!(
            members(env, &l.owner.token, l.course)
                .await
                .contains(&l.student.id.to_string())
        );
        let book = gradebook(env, &l.owner.token, l.course).await;
        assert!(rows(&book).iter().all(|(_, former)| !former));
    })
    .await;
}

#[tokio::test]
async fn a_student_without_grades_who_leaves_is_simply_gone() {
    db_test(async |pool| {
        let l = linked(pool).await;
        let env = &l.env;
        let blank = env.user("blank@x.edu", Role::Student).await;
        env.ok(
            "POST",
            &l.admin.token,
            &format!("/groups/{}/members", l.group),
            Some(json!({"user_ids": [blank.id]})),
            200,
        )
        .await;
        assert!(rows(&gradebook(env, &l.owner.token, l.course).await).len() == 2);
        env.ok(
            "POST",
            &l.admin.token,
            &format!("/groups/{}/members/remove", l.group),
            Some(json!({"user_ids": [blank.id]})),
            200,
        )
        .await;
        let seen = rows(&gradebook(env, &l.owner.token, l.course).await);
        assert_eq!(seen, [(l.student.id.to_string(), false)]);
        assert_eq!(
            grade(env, &l.owner.token, l.assignment, blank.id, "1")
                .await
                .status,
            404
        );
    })
    .await;
}

#[tokio::test]
async fn a_former_student_stays_on_the_submissions_list_with_their_attempt() {
    db_test(async |pool| {
        let l = linked(pool).await;
        let env = &l.env;
        env.ok(
            "PUT",
            &l.student.token,
            &format!("/assignments/{}/submission/draft", l.assignment),
            Some(json!({"content": essay("my answer")})),
            200,
        )
        .await;
        env.ok(
            "POST",
            &l.student.token,
            &format!("/assignments/{}/submission/submit", l.assignment),
            None,
            201,
        )
        .await;
        set_role(env, &l.admin.token, l.student.id, "teacher").await;

        let listed = env
            .ok(
                "GET",
                &l.owner.token,
                &format!("/assignments/{}/submissions", l.assignment),
                None,
                200,
            )
            .await;
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["student_id"], json!(l.student.id));
        assert_eq!(items[0]["former"], true);
        assert_eq!(items[0]["status"], "submitted");
        let attempts = env
            .ok(
                "GET",
                &l.owner.token,
                &format!("/assignments/{}/submissions/{}", l.assignment, l.student.id),
                None,
                200,
            )
            .await;
        assert_eq!(attempts["items"].as_array().unwrap().len(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_direct_student_membership_keeps_a_teacher_on_the_roster() {
    db_test(async |pool| {
        // ADR-0004: an institution teacher may join a course as a student. The roster rule for
        // direct memberships is only "active".
        let env = Env::new(pool).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let learner = env.user("learner@x.edu", Role::Teacher).await;
        let (course, _) = env.course(&owner, &[&learner]).await;
        assert!(
            members(&env, &owner.token, course)
                .await
                .contains(&learner.id.to_string())
        );
        let book = gradebook(&env, &owner.token, course).await;
        assert_eq!(rows(&book), [(learner.id.to_string(), false)]);
    })
    .await;
}

// --- group bulk writes and the rollover (ADR-0037) -------------------------------------------

async fn group_with(env: &Env, admin: &str, name: &str, students: &[Uuid]) -> Uuid {
    let group = id_of(
        &env.ok("POST", admin, "/groups", Some(json!({"name": name})), 201)
            .await,
    );
    env.ok(
        "POST",
        admin,
        &format!("/groups/{group}/members"),
        Some(json!({"user_ids": students})),
        200,
    )
    .await;
    group
}

fn ids(value: &Value) -> Vec<String> {
    let mut found: Vec<String> = value
        .as_array()
        .expect("a list")
        .iter()
        .map(|id| id.as_str().expect("an id").to_owned())
        .collect();
    found.sort();
    found
}

fn sorted(users: &[&common::Account]) -> Vec<String> {
    let mut all: Vec<String> = users.iter().map(|u| u.id.to_string()).collect();
    all.sort();
    all
}

#[tokio::test]
async fn replacing_the_members_of_a_group_adds_removes_and_audits() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let teacher = env.user("t@x.edu", Role::Teacher).await;
        let [a, b, c, d] = [
            env.user("a@x.edu", Role::Student).await,
            env.user("b@x.edu", Role::Student).await,
            env.user("c@x.edu", Role::Student).await,
            env.user("d@x.edu", Role::Student).await,
        ];
        let group = group_with(&env, &admin.token, "7-A", &[a.id, b.id]).await;
        let sub = id_of(
            &env.ok(
                "POST",
                &admin.token,
                &format!("/groups/{group}/subgroups"),
                Some(json!({"name": "English 1"})),
                201,
            )
            .await,
        );
        env.ok(
            "POST",
            &admin.token,
            &format!("/subgroups/{sub}/members"),
            Some(json!({"user_ids": [a.id, b.id]})),
            200,
        )
        .await;
        let path = format!("/groups/{group}/members");

        let done = env
            .ok(
                "PUT",
                &admin.token,
                &path,
                Some(json!({"user_ids": [b.id, c.id, d.id, c.id]})),
                200,
            )
            .await;
        assert_eq!(ids(&done["added"]), sorted(&[&c, &d]));
        assert_eq!(ids(&done["removed"]), sorted(&[&a]));
        let listed = env
            .ok("GET", &admin.token, &format!("{path}?limit=100"), None, 200)
            .await;
        let mut members: Vec<String> = listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["user_id"].as_str().unwrap().to_owned())
            .collect();
        members.sort();
        assert_eq!(members, sorted(&[&b, &c, &d]));
        let in_sub = env
            .ok(
                "GET",
                &admin.token,
                &format!("/subgroups/{sub}/members"),
                None,
                200,
            )
            .await;
        assert_eq!(
            in_sub["items"].as_array().unwrap().len(),
            1,
            "a left with the group"
        );
        let audited = audit_meta(&env.pool, "group.members.replace").await;
        assert_eq!(audited.len(), 1);
        assert_eq!(ids(&audited[0]["removed"]), sorted(&[&a]));

        // The same list again changes nothing and is not audited.
        let same = env
            .ok(
                "PUT",
                &admin.token,
                &path,
                Some(json!({"user_ids": [b.id, c.id, d.id]})),
                200,
            )
            .await;
        assert_eq!(
            (same["added"].clone(), same["removed"].clone()),
            (json!([]), json!([]))
        );
        assert_eq!(
            audit_meta(&env.pool, "group.members.replace").await.len(),
            1
        );

        // A teacher in the list rejects everything, and says where.
        let bad = env
            .call(
                "PUT",
                &admin.token,
                &path,
                Some(json!({"user_ids": [a.id, teacher.id]})),
            )
            .await;
        assert_eq!(bad.problem(422).errors[0].field, "user_ids[1]");
        let still = env
            .ok("GET", &admin.token, &format!("{path}?limit=100"), None, 200)
            .await;
        assert_eq!(
            still["items"].as_array().unwrap().len(),
            3,
            "all or nothing"
        );

        // An empty list empties the group.
        let emptied = env
            .ok(
                "PUT",
                &admin.token,
                &path,
                Some(json!({"user_ids": []})),
                200,
            )
            .await;
        assert_eq!(ids(&emptied["removed"]), sorted(&[&b, &c, &d]));

        // Admins only; unknown groups are 404.
        assert_eq!(
            env.call("PUT", &teacher.token, &path, Some(json!({"user_ids": []})))
                .await
                .status,
            403
        );
        let missing = format!("/groups/{}/members", Uuid::now_v7());
        assert_eq!(
            env.call("PUT", &admin.token, &missing, Some(json!({"user_ids": []})))
                .await
                .status,
            404
        );
    })
    .await;
}

#[tokio::test]
async fn removing_several_members_skips_the_ones_who_are_not() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let [a, b, outsider] = [
            env.user("a@x.edu", Role::Student).await,
            env.user("b@x.edu", Role::Student).await,
            env.user("o@x.edu", Role::Student).await,
        ];
        let group = group_with(&env, &admin.token, "7-A", &[a.id, b.id]).await;
        let path = format!("/groups/{group}/members/remove");

        let done = env
            .ok(
                "POST",
                &admin.token,
                &path,
                Some(json!({"user_ids": [a.id, outsider.id]})),
                200,
            )
            .await;
        assert_eq!(ids(&done["removed"]), sorted(&[&a]));
        let again = env
            .ok(
                "POST",
                &admin.token,
                &path,
                Some(json!({"user_ids": [a.id]})),
                200,
            )
            .await;
        assert_eq!(again["removed"], json!([]));
        assert_eq!(audit_meta(&env.pool, "group.members.remove").await.len(), 1);
        assert_eq!(
            env.call("POST", &a.token, &path, Some(json!({"user_ids": [b.id]})))
                .await
                .status,
            403
        );
        assert_eq!(
            env.call("POST", &admin.token, &path, Some(json!({"user_ids": []})))
                .await
                .status,
            422
        );
    })
    .await;
}

#[tokio::test]
async fn a_class_rollover_is_one_call_and_last_years_results_stay_readable() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let [old_a, old_b, new_a, new_b] = [
            env.user("oa@x.edu", Role::Student).await,
            env.user("ob@x.edu", Role::Student).await,
            env.user("na@x.edu", Role::Student).await,
            env.user("nb@x.edu", Role::Student).await,
        ];
        let (course, module) = env.course(&owner, &[]).await;
        let assignment = env.assignment(&owner, module, "10").await;
        let group = group_with(&env, &admin.token, "7-A", &[old_a.id, old_b.id]).await;
        env.ok(
            "POST",
            &owner.token,
            &format!("/courses/{course}/groups"),
            Some(json!({"group_id": group})),
            201,
        )
        .await;
        for student in [&old_a, &old_b] {
            assert_eq!(
                grade(&env, &owner.token, assignment, student.id, "8")
                    .await
                    .status,
                200
            );
        }
        publish_all(&env, &owner.token, assignment).await;

        // The documented procedure: replace the members, rename the group.
        env.ok(
            "PUT",
            &admin.token,
            &format!("/groups/{group}/members"),
            Some(json!({"user_ids": [new_a.id, new_b.id]})),
            200,
        )
        .await;
        env.ok(
            "PATCH",
            &admin.token,
            &format!("/groups/{group}"),
            Some(json!({"name": "8-A"})),
            200,
        )
        .await;

        let book = gradebook(&env, &owner.token, course).await;
        let mut seen = rows(&book);
        seen.sort();
        let mut expected = vec![
            (old_a.id.to_string(), true),
            (old_b.id.to_string(), true),
            (new_a.id.to_string(), false),
            (new_b.id.to_string(), false),
        ];
        expected.sort();
        assert_eq!(
            seen, expected,
            "last year's students are former, with their grades"
        );
        let mine = env
            .ok(
                "GET",
                &old_a.token,
                &format!("/courses/{course}/my-grades"),
                None,
                200,
            )
            .await;
        assert_eq!(mine["items"][0]["score"], "8.00");
        env.ok(
            "GET",
            &new_a.token,
            &format!("/courses/{course}"),
            None,
            200,
        )
        .await;
        assert_eq!(
            env.call("GET", &old_a.token, &format!("/courses/{course}"), None)
                .await
                .status,
            404
        );
    })
    .await;
}
