//! Every operation of the API against the callers who must not be able to use it: no session, a
//! password still to be changed, a teacher and a student of another course, a student and an
//! assistant of this one. No call may succeed, none may answer 5xx, and a course the caller cannot
//! see answers 404 whatever is asked of it (never 403, which would confirm that it exists).
//!
//! The list of operations is written out here; the test that checks it against the OpenAPI
//! document makes a new operation fail this file until someone has decided who may call it.

mod common;

use std::collections::BTreeSet;

use common::{Account, Env, db_test, essay, id_of};
use serde_json::{Value, json};
use server::identity::Role;
use uuid::Uuid;

/// The resources of one course, and the ids of a user and of the other kinds of things a request
/// can name.
struct Ctx {
    course: Uuid,
    module: Uuid,
    submodule: Uuid,
    text: Uuid,
    assignment: Uuid,
    student: Uuid,
    link: Uuid,
    group: Uuid,
    subgroup: Uuid,
    term: Uuid,
    owner: Uuid,
}

type Call = (&'static str, String, Option<Value>);

/// Every authenticated operation against the resources of `c`, with a body that is valid for it.
/// `other_user` is somebody the calls can try to add.
fn operations(c: &Ctx, other_user: Uuid) -> Vec<Call> {
    let call = |method, path: String, body| (method, path, body);
    vec![
        call("GET", format!("/courses/{}", c.course), None),
        call(
            "PATCH",
            format!("/courses/{}", c.course),
            Some(json!({"title": "hacked"})),
        ),
        call("POST", format!("/courses/{}/publish", c.course), None),
        call("DELETE", format!("/courses/{}", c.course), None),
        call("GET", format!("/courses/{}/info", c.course), None),
        call(
            "PATCH",
            format!("/courses/{}/info", c.course),
            Some(json!({"code": "X"})),
        ),
        call("POST", format!("/courses/{}/archive", c.course), None),
        call("POST", format!("/courses/{}/restore", c.course), None),
        call(
            "POST",
            format!("/courses/{}/template", c.course),
            Some(json!({})),
        ),
        // The course is not a template, so even a member that may edit it gets 404.
        call(
            "POST",
            format!("/templates/{}/courses", c.course),
            Some(json!({})),
        ),
        call("GET", format!("/courses/{}/people", c.course), None),
        call("GET", format!("/courses/{}/item-meta", c.course), None),
        call("GET", format!("/items/{}/revision", c.text), None),
        call("POST", format!("/items/{}/revision/publish", c.text), None),
        call(
            "GET",
            format!("/assignments/{}/grades/{}/history", c.assignment, c.student),
            None,
        ),
        call("GET", format!("/assignments/{}/result", c.assignment), None),
        call(
            "GET",
            format!("/assignments/{}/responses", c.assignment),
            None,
        ),
        call(
            "GET",
            format!("/courses/{}/candidates?q=zz", c.course),
            None,
        ),
        call(
            "PUT",
            format!("/courses/{}/owner", c.course),
            Some(json!({"user_id": other_user})),
        ),
        call("GET", format!("/courses/{}/members", c.course), None),
        call(
            "PUT",
            format!("/courses/{}/members/{}", c.course, other_user),
            Some(json!({"role": "assistant"})),
        ),
        call(
            "DELETE",
            format!("/courses/{}/members/{}", c.course, c.student),
            None,
        ),
        call("GET", format!("/courses/{}/groups", c.course), None),
        call(
            "POST",
            format!("/courses/{}/groups", c.course),
            Some(json!({"group_id": c.group})),
        ),
        call(
            "DELETE",
            format!("/courses/{}/groups/{}", c.course, c.link),
            None,
        ),
        call("GET", format!("/courses/{}/group-options", c.course), None),
        call("GET", format!("/courses/{}/exclusions", c.course), None),
        call(
            "DELETE",
            format!("/courses/{}/exclusions/{}", c.course, c.student),
            None,
        ),
        call("GET", format!("/courses/{}/content", c.course), None),
        call(
            "POST",
            format!("/courses/{}/modules", c.course),
            Some(json!({"title": "x"})),
        ),
        call(
            "PUT",
            format!("/courses/{}/modules/order", c.course),
            Some(json!({"ids": [c.module]})),
        ),
        call(
            "PATCH",
            format!("/modules/{}", c.module),
            Some(json!({"title": "x"})),
        ),
        call("DELETE", format!("/modules/{}", c.submodule), None),
        call(
            "POST",
            format!("/modules/{}/publish", c.module),
            Some(json!({"cascade": true})),
        ),
        call(
            "POST",
            format!("/modules/{}/items", c.module),
            Some(json!({"kind": "text", "title": "x"})),
        ),
        call(
            "PUT",
            format!("/modules/{}/items/order", c.module),
            Some(json!({"ids": [c.text, c.assignment]})),
        ),
        call("GET", format!("/items/{}", c.text), None),
        call("GET", format!("/items/{}", c.assignment), None),
        call(
            "PATCH",
            format!("/items/{}", c.text),
            Some(json!({"title": "x"})),
        ),
        call(
            "PATCH",
            format!("/items/{}", c.assignment),
            Some(json!({"max_points": "11"})),
        ),
        call("POST", format!("/items/{}/publish", c.text), None),
        call("DELETE", format!("/items/{}", c.text), None),
        call("GET", format!("/courses/{}/gradebook", c.course), None),
        call("GET", format!("/courses/{}/my-grades", c.course), None),
        call(
            "PUT",
            format!("/assignments/{}/grades/{}", c.assignment, c.student),
            Some(json!({"score": "1", "feedback": "hax"})),
        ),
        call(
            "POST",
            format!("/assignments/{}/grades/publish", c.assignment),
            Some(json!({"students": "all"})),
        ),
        call(
            "GET",
            format!("/assignments/{}/submissions", c.assignment),
            None,
        ),
        call(
            "GET",
            format!("/assignments/{}/submissions/{}", c.assignment, c.student),
            None,
        ),
        call(
            "GET",
            format!("/assignments/{}/submission", c.assignment),
            None,
        ),
        call(
            "PUT",
            format!("/assignments/{}/submission/draft", c.assignment),
            Some(json!({"content": essay("intruder")})),
        ),
        call(
            "DELETE",
            format!("/assignments/{}/submission/draft", c.assignment),
            None,
        ),
        call(
            "POST",
            format!("/assignments/{}/submission/submit", c.assignment),
            None,
        ),
        // The institution's own: admins only.
        call("GET", "/admin/institution".into(), None),
        call("GET", "/admin/users".into(), None),
        call("GET", "/admin/users/summary".into(), None),
        call("GET", format!("/admin/users/{}/facts", c.owner), None),
        call("GET", format!("/admin/groups/{}/overview", c.group), None),
        call("GET", "/admin/term-stats".into(), None),
        call("GET", "/admin/courses".into(), None),
        call("GET", "/admin/audit".into(), None),
        call(
            "POST",
            "/admin/users".into(),
            Some(json!({"email": "new@x.edu", "name": "N", "surname": "N", "role": "admin"})),
        ),
        call(
            "PATCH",
            format!("/admin/users/{}", c.owner),
            Some(json!({"role": "student"})),
        ),
        call(
            "POST",
            format!("/admin/users/{}/reset-password", c.owner),
            None,
        ),
        call("GET", "/groups".into(), None),
        call("POST", "/groups".into(), Some(json!({"name": "evil"}))),
        call("GET", format!("/groups/{}", c.group), None),
        call(
            "PATCH",
            format!("/groups/{}", c.group),
            Some(json!({"name": "evil2"})),
        ),
        call("DELETE", format!("/groups/{}", c.group), None),
        call("GET", format!("/groups/{}/members", c.group), None),
        call(
            "POST",
            format!("/groups/{}/members", c.group),
            Some(json!({"user_ids": [c.student]})),
        ),
        call(
            "PUT",
            format!("/groups/{}/members", c.group),
            Some(json!({"user_ids": []})),
        ),
        call(
            "POST",
            format!("/groups/{}/members/remove", c.group),
            Some(json!({"user_ids": [c.student]})),
        ),
        call(
            "DELETE",
            format!("/groups/{}/members/{}", c.group, c.student),
            None,
        ),
        call("GET", format!("/groups/{}/subgroups", c.group), None),
        call(
            "POST",
            format!("/groups/{}/subgroups", c.group),
            Some(json!({"name": "evil"})),
        ),
        call(
            "PATCH",
            format!("/subgroups/{}", c.subgroup),
            Some(json!({"name": "evil2"})),
        ),
        call("DELETE", format!("/subgroups/{}", c.subgroup), None),
        call("GET", format!("/subgroups/{}/members", c.subgroup), None),
        call(
            "POST",
            format!("/subgroups/{}/members", c.subgroup),
            Some(json!({"user_ids": [c.student]})),
        ),
        call(
            "DELETE",
            format!("/subgroups/{}/members/{}", c.subgroup, c.student),
            None,
        ),
        call(
            "POST",
            "/terms".into(),
            Some(json!({"name": "evil", "starts_on": "2026-01-01", "ends_on": "2026-02-01"})),
        ),
        call(
            "PATCH",
            format!("/terms/{}", c.term),
            Some(json!({"name": "evil2"})),
        ),
        call("DELETE", format!("/terms/{}", c.term), None),
    ]
}

/// Whether a path belongs to the institution (admins only) and so is not a course resource.
fn is_institutional(path: &str) -> bool {
    ["/admin", "/groups", "/subgroups", "/terms"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// A published course with a module, a sub-module, a text item, an assignment with a draft, an
/// attempt, a grade, a linked group and a term to name.
async fn build(env: &Env, admin: &Account, owner: &Account, student: &Account, tag: &str) -> Ctx {
    let (course, module) = env.course(owner, &[student]).await;
    let sub_path = format!("/courses/{course}/modules");
    let sub = id_of(
        &env.ok(
            "POST",
            &owner.token,
            &sub_path,
            Some(json!({"title": "sub", "parent_id": module})),
            201,
        )
        .await,
    );
    let items = format!("/modules/{module}/items");
    let text = id_of(
        &env.ok(
            "POST",
            &owner.token,
            &items,
            Some(json!({"kind": "text", "title": "t"})),
            201,
        )
        .await,
    );
    let assignment = env.assignment(owner, module, "10").await;
    let group = id_of(
        &env.ok(
            "POST",
            &admin.token,
            "/groups",
            Some(json!({"name": format!("g-{tag}")})),
            201,
        )
        .await,
    );
    let subgroup = id_of(
        &env.ok(
            "POST",
            &admin.token,
            &format!("/groups/{group}/subgroups"),
            Some(json!({"name": "s"})),
            201,
        )
        .await,
    );
    let link = id_of(
        &env.ok(
            "POST",
            &owner.token,
            &format!("/courses/{course}/groups"),
            Some(json!({"group_id": group})),
            201,
        )
        .await,
    );
    let term = id_of(
        &env.ok(
            "POST",
            &admin.token,
            "/terms",
            Some(json!({"name": format!("t-{tag}"), "starts_on": "2026-01-01", "ends_on": "2026-06-01"})),
            201,
        )
        .await,
    );
    // Something to leak: an attempt, a draft on top of it and a grade.
    let draft = format!("/assignments/{assignment}/submission/draft");
    env.ok(
        "PUT",
        &student.token,
        &draft,
        Some(json!({"content": essay("secret answer")})),
        200,
    )
    .await;
    env.ok(
        "POST",
        &student.token,
        &format!("/assignments/{assignment}/submission/submit"),
        None,
        201,
    )
    .await;
    env.ok(
        "PUT",
        &student.token,
        &draft,
        Some(json!({"content": essay("secret draft")})),
        200,
    )
    .await;
    env.ok(
        "PUT",
        &owner.token,
        &format!("/assignments/{assignment}/grades/{}", student.id),
        Some(json!({"score": "9", "feedback": "secret feedback"})),
        200,
    )
    .await;
    Ctx {
        course,
        module,
        submodule: sub,
        text,
        assignment,
        student: student.id,
        link,
        group,
        subgroup,
        term,
        owner: owner.id,
    }
}

#[tokio::test]
async fn no_operation_answers_a_caller_who_may_not_use_it() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let owner_a = env.user("owner-a@x.edu", Role::Teacher).await;
        let student_a = env.user("student-a@x.edu", Role::Student).await;
        let owner_b = env.user("owner-b@x.edu", Role::Teacher).await;
        let student_b = env.user("student-b@x.edu", Role::Student).await;
        let outsider_teacher = env.user("out-t@x.edu", Role::Teacher).await;
        let outsider_student = env.user("out-s@x.edu", Role::Student).await;
        let a = build(&env, &admin, &owner_a, &student_a, "a").await;
        // A second course, so that its owner is somebody strong who is not a member of the first.
        build(&env, &admin, &owner_b, &student_b, "b").await;

        let mut wrong = Vec::new();

        // No session: 401 everywhere.
        for (method, path, body) in operations(&a, outsider_student.id) {
            let request = match method {
                "GET" => env.http.get(&path),
                "POST" => env.http.post(&path),
                "PUT" => env.http.put(&path),
                "PATCH" => env.http.patch(&path),
                _ => env.http.delete(&path),
            };
            let response = match &body {
                Some(body) => request.json(body).send().await,
                None => request.send().await,
            };
            if response.status != 401 {
                wrong.push(format!(
                    "no session: {method} {path} -> {}",
                    response.status
                ));
            }
        }

        // People who are not in course A: never a success, never a 5xx, and 404 for course things.
        for (who, token) in [
            ("owner of another course", &owner_b.token),
            ("student of another course", &student_b.token),
            ("teacher outside", &outsider_teacher.token),
            ("student outside", &outsider_student.token),
        ] {
            for (method, path, body) in operations(&a, outsider_student.id) {
                let status = env.call(method, token, &path, body).await.status;
                let hidden = !is_institutional(&path);
                if !(400..500).contains(&status) || (hidden && status != 404) {
                    wrong.push(format!("{who}: {method} {path} -> {status}"));
                }
            }
        }

        // A student of the course: reads and their own submission only.
        let may = |method: &str, path: &str| {
            let reads = [
                format!("/courses/{}", a.course),
                format!("/courses/{}/members", a.course),
                format!("/courses/{}/content", a.course),
                format!("/items/{}", a.text),
                format!("/items/{}", a.assignment),
                format!("/courses/{}/my-grades", a.course),
                format!("/courses/{}/info", a.course),
                format!("/courses/{}/people", a.course),
                format!("/courses/{}/item-meta", a.course),
                format!("/assignments/{}/submission", a.assignment),
            ];
            (method == "GET" && reads.iter().any(|read| read == path))
                || path.contains("/submission/")
        };
        for (method, path, body) in operations(&a, outsider_student.id) {
            let status = env.call(method, &student_a.token, &path, body).await.status;
            if (status < 300 && !may(method, &path)) || status >= 500 {
                wrong.push(format!(
                    "student of the course: {method} {path} -> {status}"
                ));
            }
        }

        assert!(
            wrong.is_empty(),
            "{} wrong answers:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    })
    .await;
}

#[tokio::test]
async fn a_password_still_to_be_changed_and_an_assistants_root_actions_are_refused() {
    db_test(async |pool| {
        let env = Env::new(pool.clone()).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let student = env.user("student@x.edu", Role::Student).await;
        let assistant = env.user("assistant@x.edu", Role::Student).await;
        let a = build(&env, &admin, &owner, &student, "a").await;
        env.ok(
            "PUT",
            &owner.token,
            &format!("/courses/{}/members/{}", a.course, assistant.id),
            Some(json!({"role": "assistant"})),
            200,
        )
        .await;

        // The strongest role that still owes a password change may do nothing.
        let temporary = common::seed_pending_user(&pool, "pending-admin@x.edu", Role::Admin).await;
        let pending = common::login_app(&env.http, "pending-admin@x.edu", &temporary).await;
        let mut wrong = Vec::new();
        for (method, path, body) in operations(&a, student.id) {
            let status = env.call(method, &pending, &path, body).await.status;
            if status != 403 {
                wrong.push(format!("pending password: {method} {path} -> {status}"));
            }
        }

        // An assistant runs the course day to day; the root actions stay with teachers and owners,
        // and so does choosing and linking a group (ADR-0040).
        let course = a.course;
        let root: Vec<Call> = vec![
            ("POST", format!("/courses/{course}/publish"), None),
            (
                "PUT",
                format!("/courses/{course}/members/{}", assistant.id),
                Some(json!({"role": "teacher"})),
            ),
            (
                "PUT",
                format!("/courses/{course}/members/{}", student.id),
                Some(json!({"role": "teacher"})),
            ),
            (
                "PUT",
                format!("/courses/{course}/members/{}", student.id),
                Some(json!({"role": "assistant"})),
            ),
            (
                "DELETE",
                format!("/courses/{course}/members/{}", owner.id),
                None,
            ),
            (
                "PUT",
                format!("/courses/{course}/owner"),
                Some(json!({"user_id": owner.id})),
            ),
            ("GET", format!("/courses/{course}/group-options"), None),
            (
                "POST",
                format!("/courses/{course}/groups"),
                Some(json!({"group_id": a.group})),
            ),
            (
                "DELETE",
                format!("/courses/{course}/groups/{}", a.link),
                None,
            ),
        ];
        for (method, path, body) in root {
            let status = env
                .call(method, &assistant.token, &path, body.clone())
                .await
                .status;
            if status != 403 {
                wrong.push(format!("assistant: {method} {path} {body:?} -> {status}"));
            }
        }
        assert!(
            wrong.is_empty(),
            "{} wrong answers:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    })
    .await;
}

/// Linking a group puts its students on the roster, which shows their names: choosing a group is
/// for the owner, teachers and admins, and an assistant who is a student cannot read another class
/// that way (ADR-0040).
#[tokio::test]
async fn an_assistant_cannot_read_the_members_of_a_group_by_linking_it() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let admin = env.user("admin@x.edu", Role::Admin).await;
        let owner = env.user("owner@x.edu", Role::Teacher).await;
        let assistant = env.user("ta@x.edu", Role::Student).await;
        let victim = env.user("victim@x.edu", Role::Student).await;
        let (course, _module) = env.course(&owner, &[]).await;
        env.ok(
            "PUT",
            &owner.token,
            &format!("/courses/{course}/members/{}", assistant.id),
            Some(json!({"role": "assistant"})),
            200,
        )
        .await;
        let group = id_of(
            &env.ok(
                "POST",
                &admin.token,
                "/groups",
                Some(json!({"name": "11-B"})),
                201,
            )
            .await,
        );
        env.ok(
            "POST",
            &admin.token,
            &format!("/groups/{group}/members"),
            Some(json!({"user_ids": [victim.id]})),
            200,
        )
        .await;

        let options = env
            .call(
                "GET",
                &assistant.token,
                &format!("/courses/{course}/group-options"),
                None,
            )
            .await;
        assert_eq!(options.status, 403);
        let link = env
            .call(
                "POST",
                &assistant.token,
                &format!("/courses/{course}/groups"),
                Some(json!({"group_id": group})),
            )
            .await;
        assert_eq!(link.status, 403);
        let book = env
            .ok(
                "GET",
                &assistant.token,
                &format!("/courses/{course}/gradebook"),
                None,
                200,
            )
            .await;
        assert_eq!(book["items"], json!([]), "nobody of 11-B is on the course");
        // The owner can: that is the job.
        env.ok(
            "POST",
            &owner.token,
            &format!("/courses/{course}/groups"),
            Some(json!({"group_id": group})),
            201,
        )
        .await;
    })
    .await;
}

/// The operations written out above are the operations the API has: a new route fails this test
/// until it is added to the list and somebody has decided who may call it.
#[tokio::test]
async fn the_list_of_operations_is_the_api() {
    db_test(async |pool| {
        let env = Env::new(pool).await;
        let spec: Value = env.http.get("/openapi.json").send().await.json();
        let mut in_api = BTreeSet::new();
        for (path, item) in spec["paths"].as_object().unwrap() {
            let path = path.trim_start_matches("/api/v1");
            for method in item.as_object().unwrap().keys() {
                in_api.insert(format!("{} {path}", method.to_uppercase()));
            }
        }
        // Operations that need no sign-in or only a session of one's own are not in the list above.
        let own: BTreeSet<String> = [
            "GET /health",
            "GET /institution",
            "POST /auth/login",
            "POST /auth/register",
            "POST /auth/verify-email",
            "POST /auth/password-reset",
            "POST /auth/password-reset/confirm",
            "POST /auth/google/start",
            "POST /auth/google/callback",
            "POST /auth/app/exchange",
            "POST /auth/logout",
            "GET /me",
            "POST /me/password",
            "GET /me/sessions",
            "DELETE /me/sessions/{id}",
            "GET /me/courses",
            "GET /me/templates",
            "GET /me/course-cards",
            "GET /me/work",
            "GET /me/teaching",
            "GET /search",
            "POST /courses",
            "GET /editor/profiles",
            "GET /terms",
            "GET /terms/{id}",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();

        let any = Uuid::nil();
        let c = Ctx {
            course: any,
            module: any,
            submodule: any,
            text: any,
            assignment: any,
            student: any,
            link: any,
            group: any,
            subgroup: any,
            term: any,
            owner: any,
        };
        let listed: BTreeSet<String> = operations(&c, any)
            .into_iter()
            .map(|(method, path, _)| {
                // Back from the concrete nil ids to the templates of the document.
                let template = in_api
                    .iter()
                    .find(|api| {
                        let (api_method, api_path) = api.split_once(' ').unwrap();
                        api_method == method && matches_template(api_path, &path)
                    })
                    .cloned();
                template.unwrap_or(format!("{method} {path} (not in the API)"))
            })
            .collect();
        let missing: Vec<_> = in_api
            .difference(&listed)
            .filter(|op| !own.contains(*op))
            .collect();
        let unknown: Vec<_> = listed.difference(&in_api).collect();
        assert!(
            missing.is_empty(),
            "operations nobody checked:\n{missing:#?}"
        );
        assert!(
            unknown.is_empty(),
            "listed but not in the API:\n{unknown:#?}"
        );
    })
    .await;
}

/// Does `path` (with nil ids) fit `template` (`/courses/{id}/members/{user_id}`)?
fn matches_template(template: &str, path: &str) -> bool {
    // A query string is part of the call, not of the path.
    let path = path.split('?').next().unwrap_or(path);
    let nil = Uuid::nil().to_string();
    let (mut a, mut b) = (template.split('/'), path.split('/'));
    loop {
        match (a.next(), b.next()) {
            (None, None) => return true,
            (Some(t), Some(p)) if t == p || (t.starts_with('{') && p == nil) => {}
            _ => return false,
        }
    }
}
