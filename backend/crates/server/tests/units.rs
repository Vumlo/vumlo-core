//! The unit grant over HTTP (ADR-0094): what a role on a unit lets its holder do with the courses
//! below it, and that a change of the role or of its abilities is seen by the very next request.
//! Who may call each route is in `tests/authorization.rs`; the unit endpoints themselves are in
//! `tests/org.rs` (that file also covers a student refused a role).

mod common;

use ::common::Role;
use common::{Account, Env, db_test, id_of};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Units A > B and S (beside A); Pat manages A, Vic views A, Sam manages S. Tom, a teacher, made
/// the draft course K and an admin put it in B.
struct World {
    env: Env,
    admin: Account,
    tom: Account,
    pat: Account,
    vic: Account,
    sam: Account,
    a: Uuid,
    b: Uuid,
    k: Uuid,
}

async fn world(pool: PgPool) -> World {
    let env = Env::new(pool).await;
    let admin = env.user("admin@x.edu", Role::Admin).await;
    let tom = env.user("tom@x.edu", Role::Teacher).await;
    let pat = env.user("pat@x.edu", Role::Teacher).await;
    let vic = env.user("vic@x.edu", Role::Teacher).await;
    let sam = env.user("sam@x.edu", Role::Teacher).await;
    let unit = async |name: &str, parent: Option<Uuid>| {
        let body = Some(json!({"name": name, "parent_id": parent}));
        id_of(&env.ok("POST", &admin.token, "/units", body, 201).await)
    };
    let a = unit("A", None).await;
    let b = unit("B", Some(a)).await;
    let s = unit("S", None).await;
    for (unit, who, role) in [
        (a, &pat, "manager"),
        (a, &vic, "viewer"),
        (s, &sam, "manager"),
    ] {
        let path = format!("/units/{unit}/roles/{}", who.id);
        env.ok(
            "PUT",
            &admin.token,
            &path,
            Some(json!({ "role": role })),
            204,
        )
        .await;
    }
    let k = id_of(
        &env.ok(
            "POST",
            &tom.token,
            "/courses",
            Some(json!({"title": "K"})),
            201,
        )
        .await,
    );
    env.ok(
        "PUT",
        &admin.token,
        &format!("/courses/{k}/unit"),
        Some(json!({"unit_id": b})),
        200,
    )
    .await;
    World {
        env,
        admin,
        tom,
        pat,
        vic,
        sam,
        a,
        b,
        k,
    }
}

impl World {
    async fn status(
        &self,
        who: &Account,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> u16 {
        self.env.call(method, &who.token, path, body).await.status
    }

    /// The `course.publish` rows of K: `(actor, meta)`.
    async fn publishes(&self) -> Vec<(Option<Uuid>, serde_json::Value)> {
        sqlx::query!(
            "SELECT actor_id, meta FROM events.audit_log
             WHERE action = 'course.publish' AND resource_id = $1 ORDER BY at, id",
            self.k
        )
        .fetch_all(&self.env.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.actor_id, r.meta))
        .collect()
    }

    async fn audit_rows(&self) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE resource_id = $1"#,
            self.k
        )
        .fetch_one(&self.env.pool)
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn a_manager_runs_a_course_of_their_unit_without_being_on_it() {
    db_test(async |pool| {
        let w = world(pool).await;
        let k = w.k;

        // Manager of A, K is in B below it: the roster is theirs, the content is not.
        let seen = w
            .env
            .ok("GET", &w.pat.token, &format!("/courses/{k}"), None, 200)
            .await;
        assert_eq!(seen["can"]["manage_roster"], json!(true));
        assert_eq!(seen["can"]["edit_content"], json!(false));

        let publish = w
            .status(&w.pat, "POST", &format!("/courses/{k}/publish"), None)
            .await;
        assert_eq!(publish, 200);
        let title = json!({"title": "Renamed"});
        let patch = w
            .status(&w.pat, "PATCH", &format!("/courses/{k}"), Some(title))
            .await;
        assert_eq!(patch, 200);

        // What only the owner and an admin do stays refused, and the course is not hidden.
        let delete = w
            .status(&w.pat, "DELETE", &format!("/courses/{k}"), None)
            .await;
        assert_eq!(delete, 403);
        let owner = json!({"user_id": w.pat.id});
        let take = w
            .status(&w.pat, "PUT", &format!("/courses/{k}/owner"), Some(owner))
            .await;
        assert_eq!(take, 403);
    })
    .await;
}

#[tokio::test]
async fn a_managers_change_is_audited_and_the_teachers_is_not() {
    db_test(async |pool| {
        let w = world(pool).await;
        let k = w.k;
        w.env
            .ok(
                "POST",
                &w.pat.token,
                &format!("/courses/{k}/publish"),
                None,
                200,
            )
            .await;

        let rows = w.publishes().await;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].0, Some(w.pat.id));
        assert_eq!(rows[0].1, json!({"via": "unit", "unit_id": w.b}));

        // The owner's own change is not an exception to anything, so it leaves no row.
        let before = w.audit_rows().await;
        w.env
            .ok(
                "PATCH",
                &w.tom.token,
                &format!("/courses/{k}"),
                Some(json!({"title": "Tom's title"})),
                200,
            )
            .await;
        assert_eq!(w.audit_rows().await, before);
    })
    .await;
}

#[tokio::test]
async fn switching_an_ability_off_takes_effect_on_the_next_request() {
    db_test(async |pool| {
        let w = world(pool).await;
        let book = format!("/courses/{}/gradebook", w.k);
        assert_eq!(w.status(&w.vic, "GET", &book, None).await, 200);

        w.env
            .ok(
                "PUT",
                &w.admin.token,
                "/admin/unit-roles/viewer",
                Some(json!({"abilities": []})),
                200,
            )
            .await;

        // The role still shows them the course; only the ability is gone.
        assert_eq!(w.status(&w.vic, "GET", &book, None).await, 403);
        let course = format!("/courses/{}", w.k);
        assert_eq!(w.status(&w.vic, "GET", &course, None).await, 200);
    })
    .await;
}

/// The counts of what to grade on the item facts are about submissions: a viewer sees them only
/// while their role has `view_grades`.
#[tokio::test]
async fn the_counts_to_grade_need_view_grades() {
    db_test(async |pool| {
        let w = world(pool).await;
        let (course, module) = w.env.course(&w.tom, &[]).await;
        let item = w.env.assignment(&w.tom, module, "10").await;
        w.env
            .ok(
                "PUT",
                &w.admin.token,
                &format!("/courses/{course}/unit"),
                Some(json!({"unit_id": w.b})),
                200,
            )
            .await;
        let meta = format!("/courses/{course}/item-meta");
        let submitted = async || {
            w.env.ok("GET", &w.vic.token, &meta, None, 200).await["items"][item.to_string()]
                ["submitted"]
                .clone()
        };
        assert_eq!(submitted().await, json!(0));

        w.env
            .ok(
                "PUT",
                &w.admin.token,
                "/admin/unit-roles/viewer",
                Some(json!({"abilities": []})),
                200,
            )
            .await;
        assert_eq!(submitted().await, serde_json::Value::Null);
    })
    .await;
}

#[tokio::test]
async fn revoking_a_role_takes_effect_on_the_next_request() {
    db_test(async |pool| {
        let w = world(pool).await;
        let course = format!("/courses/{}", w.k);
        assert_eq!(w.status(&w.pat, "GET", &course, None).await, 200);

        w.env
            .ok(
                "DELETE",
                &w.admin.token,
                &format!("/units/{}/roles/{}", w.a, w.pat.id),
                None,
                204,
            )
            .await;

        assert_eq!(w.status(&w.pat, "GET", &course, None).await, 404);
    })
    .await;
}

#[tokio::test]
async fn a_sibling_unit_sees_nothing() {
    db_test(async |pool| {
        let w = world(pool).await;
        let course = format!("/courses/{}", w.k);
        assert_eq!(w.status(&w.sam, "GET", &course, None).await, 404);
        let listing = format!("/units/{}/courses", w.b);
        assert_eq!(w.status(&w.sam, "GET", &listing, None).await, 404);
    })
    .await;
}
