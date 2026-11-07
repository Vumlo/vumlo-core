//! ADR-0077: a plugin's reports on a submitted attempt, an assignment and a course, kept under keys
//! of its own, replaced by writing again, removed by deleting, and read by the staff who may see the
//! subject. A plugin writes only in the courses it reaches.

mod common;

use ::common::Role;
use access::{CourseScope, Scope};
use common::{Account, Env, db_test, id_of, spawn_app, test_settings};
use plugins::{Installed, parse_manifest};
use serde_json::{Value, json};
use server::app::AppState;
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::Secret;

struct World {
    env: Env,
    state: AppState,
    teacher: Account,
    student: Account,
    course: Uuid,
    assignment: Uuid,
    submission: Uuid,
    other_course: Uuid,
    other_assignment: Uuid,
}

async fn world(pool: PgPool) -> World {
    let mut settings = test_settings();
    settings.plugins.secret_key = Secret::new("0123456789abcdef0123456789abcdef");
    let (base, state) = spawn_app(pool.clone(), settings).await;
    let env = Env {
        http: common::Http::new(&base),
        pool,
        base,
    };
    let teacher = env.user("teacher@x.edu", Role::Teacher).await;
    let student = env.user("student@x.edu", Role::Student).await;
    let (course, module) = env.course(&teacher, &[&student]).await;
    let assignment = env.assignment(&teacher, module, "10").await;
    let (other_course, other_module) = env.course(&teacher, &[&student]).await;
    let other_assignment = env.assignment(&teacher, other_module, "10").await;
    let path = format!("/assignments/{assignment}/submission");
    env.ok(
        "PUT",
        &student.token,
        &format!("{path}/draft"),
        Some(json!({"content": common::essay("some words")})),
        200,
    )
    .await;
    let submission = id_of(
        &env.ok("POST", &student.token, &format!("{path}/submit"), None, 201)
            .await,
    );
    World {
        env,
        state,
        teacher,
        student,
        course,
        assignment,
        submission,
        other_course,
        other_assignment,
    }
}

impl World {
    async fn plugin(&self, id: &str, scopes: &[Scope], courses: CourseScope) -> Installed {
        let manifest = parse_manifest(
            json!({"id": id, "version": "1", "api": "1", "scopes": Scope::ALL.map(Scope::as_str)})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        let key = self.state.plugins.key.as_deref().expect("a plugin key");
        let mut conn = self.state.db.acquire().await.unwrap();
        plugins::install(
            &mut conn,
            key,
            "http://127.0.0.1:9",
            &manifest,
            scopes,
            &courses,
        )
        .await
        .unwrap()
    }

    async fn writer(&self, id: &str) -> Installed {
        self.plugin(id, &[Scope::ReportsWrite], CourseScope::All)
            .await
    }

    async fn send(
        &self,
        who: &Installed,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> common::Resp {
        let http = common::Http::new(&self.env.base);
        let url = format!("/plugin-api{path}");
        let request = match method {
            "PUT" => http.put(&url),
            "DELETE" => http.delete(&url),
            _ => http.get(&url),
        }
        .bearer(who.token.expose());
        match body {
            Some(body) => request.json(&body).send().await,
            None => request.send().await,
        }
    }

    async fn read(&self, who: &Account, path: &str) -> common::Resp {
        self.env.call("GET", &who.token, path, None).await
    }

    /// The reports of the subject as staff read them: `(plugin, key, summary)`.
    async fn reports(&self, path: &str) -> Vec<(String, String, String)> {
        let body = self
            .env
            .ok("GET", &self.teacher.token, path, None, 200)
            .await;
        body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["plugin_id"].as_str().unwrap().to_owned(),
                    r["key"].as_str().unwrap().to_owned(),
                    r["summary"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }
}

fn report(summary: &str) -> Value {
    json!({"summary": summary, "data": {"n": 1}})
}

#[tokio::test]
async fn a_report_is_kept_per_key_on_each_kind_of_subject_and_staff_read_them() {
    db_test(async |pool| {
        let w = world(pool).await;
        let p = w.writer("p").await;
        let q = w.writer("q").await;
        let on_submission = format!("/submissions/{}/reports", w.submission);
        let on_assignment = format!("/assignments/{}/reports", w.assignment);
        let on_course = format!("/courses/{}/reports", w.course);

        for base in [&on_submission, &on_assignment, &on_course] {
            for (who, key, summary) in [
                (&p, "first", "one"),
                (&p, "second", "two"),
                (&q, "first", "other"),
            ] {
                let sent = w
                    .send(who, "PUT", &format!("{base}/{key}"), Some(report(summary)))
                    .await;
                assert_eq!(
                    sent.status,
                    204,
                    "{base}/{key}: {}",
                    String::from_utf8_lossy(&sent.body)
                );
            }
        }
        let rows = |plugin: &str, key: &str, summary: &str| {
            (plugin.to_owned(), key.to_owned(), summary.to_owned())
        };
        let expected = vec![
            rows("p", "first", "one"),
            rows("p", "second", "two"),
            rows("q", "first", "other"),
        ];
        // Each subject has its own: nothing leaks from one level to the next.
        assert_eq!(
            w.reports(&format!("/submissions/{}/plugin-reports", w.submission))
                .await,
            expected
        );
        assert_eq!(
            w.reports(&format!("/assignments/{}/plugin-reports", w.assignment))
                .await,
            expected
        );
        assert_eq!(
            w.reports(&format!("/courses/{}/plugin-reports", w.course))
                .await,
            expected
        );
        assert!(
            w.reports(&format!("/courses/{}/plugin-reports", w.other_course))
                .await
                .is_empty()
        );

        // Writing again under a key replaces it; another key is left alone; the data comes back.
        w.send(
            &p,
            "PUT",
            &format!("{on_course}/first"),
            Some(json!({"summary": "changed", "data": {"rows": [1, 2]}})),
        )
        .await;
        let body = w
            .env
            .ok(
                "GET",
                &w.teacher.token,
                &format!("/courses/{}/plugin-reports", w.course),
                None,
                200,
            )
            .await;
        let first = &body["items"][0];
        assert_eq!(first["summary"], "changed");
        assert_eq!(first["data"], json!({"rows": [1, 2]}));
        assert_eq!(body["items"].as_array().unwrap().len(), 3);

        // Deleting removes only that key of that plugin; deleting again is a 404.
        assert_eq!(
            w.send(&p, "DELETE", &format!("{on_course}/first"), None)
                .await
                .status,
            204
        );
        w.send(&p, "DELETE", &format!("{on_course}/first"), None)
            .await
            .problem(404);
        assert_eq!(
            w.reports(&format!("/courses/{}/plugin-reports", w.course))
                .await,
            vec![rows("p", "second", "two"), rows("q", "first", "other")]
        );
    })
    .await;
}

#[tokio::test]
async fn a_report_is_refused_without_the_scope_outside_the_courses_and_for_what_is_not_there() {
    db_test(async |pool| {
        let w = world(pool).await;
        let reader = w
            .plugin("reader", &[Scope::SubmissionsRead], CourseScope::All)
            .await;
        let here = w
            .plugin(
                "here",
                &[Scope::ReportsWrite],
                CourseScope::Only(vec![w.course]),
            )
            .await;
        let all = w.writer("all").await;
        let ghost = Uuid::now_v7();

        for path in [
            format!("/submissions/{}/reports/k", w.submission),
            format!("/assignments/{}/reports/k", w.assignment),
            format!("/courses/{}/reports/k", w.course),
            // Whether the id is there makes no difference without the scope.
            format!("/courses/{ghost}/reports/k"),
        ] {
            w.send(&reader, "PUT", &path, Some(report("x")))
                .await
                .problem(403);
            w.send(&reader, "DELETE", &path, None).await.problem(403);
        }
        // A plugin with the scope and a course gets in; the other course is 403.
        for path in [
            format!("/assignments/{}/reports/k", w.assignment),
            format!("/courses/{}/reports/k", w.course),
        ] {
            assert_eq!(
                w.send(&here, "PUT", &path, Some(report("x"))).await.status,
                204
            );
        }
        for path in [
            format!("/assignments/{}/reports/k", w.other_assignment),
            format!("/courses/{}/reports/k", w.other_course),
        ] {
            w.send(&here, "PUT", &path, Some(report("x")))
                .await
                .problem(403);
            w.send(&here, "DELETE", &path, None).await.problem(403);
        }
        assert!(
            w.reports(&format!("/courses/{}/plugin-reports", w.other_course))
                .await
                .is_empty()
        );
        // Not there: 404 for a plugin that reaches every course.
        for path in [
            format!("/submissions/{ghost}/reports/k"),
            format!("/assignments/{ghost}/reports/k"),
            format!("/courses/{ghost}/reports/k"),
        ] {
            w.send(&all, "PUT", &path, Some(report("x")))
                .await
                .problem(404);
            w.send(&all, "DELETE", &path, None).await.problem(404);
        }
        // A draft of the student is no attempt to report on.
        w.env
            .ok(
                "PUT",
                &w.student.token,
                &format!("/assignments/{}/submission/draft", w.assignment),
                Some(json!({"content": common::essay("draft only")})),
                200,
            )
            .await;
        let draft: Uuid =
            sqlx::query_scalar("SELECT id FROM assessment.submissions WHERE status = 'draft'")
                .fetch_one(&w.env.pool)
                .await
                .unwrap();
        w.send(
            &all,
            "PUT",
            &format!("/submissions/{draft}/reports/k"),
            Some(report("x")),
        )
        .await
        .problem(404);
    })
    .await;
}

#[tokio::test]
async fn a_key_a_summary_and_the_number_of_keys_are_checked() {
    db_test(async |pool| {
        let w = world(pool).await;
        let p = w.writer("p").await;
        let base = format!("/courses/{}/reports", w.course);
        for key in ["Upper", "1st", "with space", "has.dot", &"k".repeat(41)] {
            w.send(&p, "PUT", &format!("{base}/{key}"), Some(report("x")))
                .await
                .problem(422);
        }
        w.send(
            &p,
            "PUT",
            &format!("{base}/ok"),
            Some(json!({"summary": "", "data": {}})),
        )
        .await
        .problem(422);
        w.send(
            &p,
            "PUT",
            &format!("{base}/ok"),
            Some(json!({"summary": "x", "data": [1]})),
        )
        .await
        .problem(422);
        // Twenty keys on a subject; the twenty-first is refused, replacing one is not.
        for n in 0..20 {
            assert_eq!(
                w.send(&p, "PUT", &format!("{base}/k{n}"), Some(report("x")))
                    .await
                    .status,
                204
            );
        }
        let problem = w
            .send(&p, "PUT", &format!("{base}/more"), Some(report("x")))
            .await
            .problem(422);
        assert_eq!(problem.errors[0].field, "key");
        assert_eq!(
            w.send(&p, "PUT", &format!("{base}/k3"), Some(report("again")))
                .await
                .status,
            204
        );
        // Another plugin has its own twenty, and the same plugin its own on another subject.
        let q = w.writer("q").await;
        assert_eq!(
            w.send(&q, "PUT", &format!("{base}/more"), Some(report("x")))
                .await
                .status,
            204
        );
        assert_eq!(
            w.send(
                &p,
                "PUT",
                &format!("/assignments/{}/reports/more", w.assignment),
                Some(report("x"))
            )
            .await
            .status,
            204
        );
    })
    .await;
}

#[tokio::test]
async fn only_staff_who_may_see_the_subject_read_its_reports() {
    db_test(async |pool| {
        let w = world(pool).await;
        let p = w.writer("p").await;
        let outsider = w.env.user("outsider@x.edu", Role::Teacher).await;
        for path in [
            format!("/submissions/{}/reports/k", w.submission),
            format!("/assignments/{}/reports/k", w.assignment),
            format!("/courses/{}/reports/k", w.course),
        ] {
            w.send(&p, "PUT", &path, Some(report("secret"))).await;
        }
        for path in [
            format!("/submissions/{}/plugin-reports", w.submission),
            format!("/assignments/{}/plugin-reports", w.assignment),
            format!("/courses/{}/plugin-reports", w.course),
        ] {
            // The student of the course sees the course and the assignment, but not what is said
            // about the class's work; a teacher of no course does not know it is there.
            assert_eq!(w.read(&w.student, &path).await.status, 403, "{path}");
            w.read(&outsider, &path).await.problem(404);
            assert_eq!(w.read(&w.teacher, &path).await.status, 200, "{path}");
        }
    })
    .await;
}

#[tokio::test]
async fn a_report_goes_with_what_it_is_about() {
    db_test(async |pool| {
        let w = world(pool).await;
        let p = w.writer("p").await;
        for path in [
            format!("/submissions/{}/reports/k", w.submission),
            format!("/assignments/{}/reports/k", w.assignment),
            format!("/courses/{}/reports/k", w.other_course),
        ] {
            assert_eq!(
                w.send(&p, "PUT", &path, Some(report("x"))).await.status,
                204
            );
        }
        let count = async || -> i64 {
            sqlx::query_scalar("SELECT count(*) FROM plugins.reports")
                .fetch_one(&w.env.pool)
                .await
                .unwrap()
        };
        assert_eq!(count().await, 3);
        // The foreign keys cascade: the subject goes, its reports go in the same statement.
        sqlx::query("DELETE FROM assessment.submissions WHERE id = $1")
            .bind(w.submission)
            .execute(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(count().await, 2);
        sqlx::query("DELETE FROM content.items WHERE id = $1")
            .bind(w.assignment)
            .execute(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(count().await, 1);
        sqlx::query("DELETE FROM teaching.courses WHERE id = $1")
            .bind(w.other_course)
            .execute(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(count().await, 0);
        // And a plugin that is removed takes its reports with it.
        w.send(
            &p,
            "PUT",
            &format!("/courses/{}/reports/k", w.course),
            Some(report("x")),
        )
        .await;
        let mut conn = w.state.db.acquire().await.unwrap();
        plugins::remove(&mut conn, "p").await.unwrap();
        assert_eq!(count().await, 0);
    })
    .await;
}
