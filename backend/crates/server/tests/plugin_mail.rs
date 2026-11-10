//! ADR-0084 to ADR-0086: a plugin with `mail:send` mails people of the courses it reaches. It names
//! them by id, never sees an address, is limited per call and per person, and every person can turn
//! one kind of mail from one plugin off with the link in the footer.

mod common;

use ::common::Role;
use access::{CourseScope, Scope};
use common::{Account, Env, Mailbox, db_test, deliver_mail, spawn_app};
use plugins::{Installed, parse_manifest};
use serde_json::{Value, json};
use server::app::AppState;
use sqlx::PgPool;
use uuid::Uuid;
use vumlo_config::{MailSettings, Secret};

const REASONS: [&str; 5] = [
    "opted_out",
    "no_address",
    "duplicate",
    "disabled",
    "rate_limited",
];

struct World {
    env: Env,
    state: AppState,
    mailbox: Mailbox,
    teacher: Account,
    assistant: Account,
    s1: Account,
    s2: Account,
    sb: Account,
    course: Uuid,
    other_course: Uuid,
}

async fn world(pool: PgPool, tune: impl FnOnce(&mut MailSettings)) -> World {
    let mailbox = Mailbox::new();
    let mut settings = mailbox.settings();
    settings.plugins.secret_key = Secret::new("0123456789abcdef0123456789abcdef");
    tune(settings.mail.as_mut().expect("the mailbox sets [mail]"));
    world_with(pool, mailbox, settings).await
}

async fn world_with(pool: PgPool, mailbox: Mailbox, settings: vumlo_config::Settings) -> World {
    let (base, state) = spawn_app(pool.clone(), settings).await;
    let env = Env {
        http: common::Http::new(&base),
        pool,
        base,
    };
    let teacher = env.user("teacher@lms.test", Role::Teacher).await;
    let assistant = env.user("assistant@lms.test", Role::Student).await;
    let s1 = env.user("s1@lms.test", Role::Student).await;
    let s2 = env.user("s2@lms.test", Role::Student).await;
    let sb = env.user("sb@lms.test", Role::Student).await;
    let (course, _) = env.course(&teacher, &[&s1, &s2]).await;
    env.ok(
        "PUT",
        &teacher.token,
        &format!("/courses/{course}/members/{}", assistant.id),
        Some(json!({"role": "assistant"})),
        200,
    )
    .await;
    let (other_course, _) = env.course(&teacher, &[&sb]).await;
    World {
        env,
        state,
        mailbox,
        teacher,
        assistant,
        s1,
        s2,
        sb,
        course,
        other_course,
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

    /// A plugin that may send mail to every course.
    async fn mailer(&self, id: &str) -> Installed {
        self.plugin(id, &[Scope::MailSend], CourseScope::All).await
    }

    async fn send(&self, who: &Installed, body: &Value) -> common::Resp {
        common::Http::new(&self.env.base)
            .post("/plugin-api/mail")
            .bearer(who.token.expose())
            .json(body)
            .send()
            .await
    }

    async fn delivered(&self) -> Vec<String> {
        deliver_mail(&self.state).await;
        self.mailbox.messages()
    }

    async fn count(&self, state: &str) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM mail.messages WHERE state = $1"#,
            state
        )
        .fetch_one(&self.env.pool)
        .await
        .unwrap()
    }
}

fn to_user(user: Uuid) -> Value {
    json!({"user_id": user})
}

fn to_course(course: Uuid, audience: &str) -> Value {
    json!({"course_id": course, "audience": audience})
}

fn mail(to: Value) -> Value {
    json!({"to": to, "kind": "deadline_changed", "subject": "Deadline moved",
           "text": "The deadline is now Friday."})
}

fn reasons(response: &common::Resp) -> Vec<u64> {
    let body = response.json();
    REASONS
        .iter()
        .map(|key| body["skipped_reasons"][key].as_u64().expect(key))
        .collect()
}

#[tokio::test]
async fn a_plugin_mails_one_person_and_the_mail_has_the_footer_the_headers_and_the_link() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w.mailer("deadline-alert").await;
        let mut body = mail(to_user(w.s1.id));
        body["link_path"] = json!("/courses/42");
        body["dedupe_key"] = json!("assignment-7");

        let response = w.send(&plugin, &body).await;

        assert_eq!(
            response.status,
            202,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert_eq!(
            response.json(),
            json!({"accepted": 1, "skipped": 0, "skipped_reasons": {
                "opted_out": 0, "no_address": 0, "duplicate": 0, "disabled": 0,
                "rate_limited": 0}})
        );
        assert!(!String::from_utf8_lossy(&response.body).contains("s1@lms.test"));
        // Nothing is sent inside the request: the worker does it.
        assert!(w.mailbox.messages().is_empty());
        assert_eq!(w.count("pending").await, 1);

        let messages = w.delivered().await;
        assert_eq!(messages.len(), 1);
        let text = &messages[0];
        for needle in [
            "s1@lms.test",
            "no-reply@lms.test",
            "Subject: Deadline moved",
            "The deadline is now Friday.",
            "https://lms.test/courses/42",
            "deadline-alert",
            "https://lms.test/api/v1/mail/opt-out/",
            "List-Unsubscribe: <https://lms.test/api/v1/mail/opt-out/",
            "List-Unsubscribe-Post: List-Unsubscribe=One-Click",
            "X-Vumlo-Plugin: deadline-alert",
            "Auto-Submitted: auto-generated",
        ] {
            assert!(text.contains(needle), "{needle}: {text}");
        }
        assert_eq!(w.count("sent").await, 1);
    })
    .await;
}

#[tokio::test]
async fn an_audience_is_the_students_the_teachers_with_assistants_or_everybody() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w.mailer("p").await;
        for (audience, expected) in [("students", 2), ("teachers", 2), ("all", 4)] {
            let body = mail(to_course(w.course, audience));
            let response = w.send(&plugin, &body).await;
            assert_eq!(response.status, 202);
            assert_eq!(response.json()["accepted"], expected, "{audience}");
            // The next audience is another kind, so it is not a duplicate of the last.
        }
        let recipients: Vec<String> = sqlx::query_scalar!(
            r#"SELECT to_address AS "to_address!" FROM mail.messages ORDER BY to_address"#
        )
        .fetch_all(&w.env.pool)
        .await
        .unwrap();
        // The other course's student and the course owner (who is on no roster) never get any.
        assert!(!recipients.iter().any(|a| a == "sb@lms.test"));
        let teachers: Vec<_> = recipients
            .iter()
            .filter(|a| a.starts_with("assistant@"))
            .collect();
        assert_eq!(
            teachers.len(),
            2,
            "the assistant is in `teachers` and `all`"
        );
        let _ = (&w.s2, &w.sb);
    })
    .await;
}

#[tokio::test]
async fn the_answers_come_in_the_order_token_body_scope_configuration_target() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let http = common::Http::new(&w.env.base);
        let good = mail(to_user(w.s1.id));

        // 401: no token, and a token nobody has.
        for request in [
            http.post("/plugin-api/mail").json(&good),
            http.post("/plugin-api/mail")
                .bearer("vplg_nonsense")
                .json(&good),
        ] {
            assert_eq!(request.send().await.status, 401);
        }

        // 403: a plugin that was not granted the scope, with a good body.
        let reader = w
            .plugin("reader", &[Scope::ReportsWrite], CourseScope::All)
            .await;
        assert_eq!(w.send(&reader, &good).await.status, 403);

        // 422 comes before 403 when the body is bad.
        let mut bad = good.clone();
        bad["subject"] = json!("two\nlines");
        let response = w.send(&reader, &bad).await;
        let problem = response.problem(422);
        assert_eq!(problem.errors[0].field, "subject");

        let plugin = w.mailer("p").await;
        for (field, value) in [
            ("kind", json!("Bad Kind")),
            ("kind", json!("")),
            ("subject", json!("")),
            ("text", json!("")),
            ("text", json!("bell\u{7}")),
            ("link_path", json!("https://evil.test/")),
            ("link_path", json!("//evil.test/")),
            ("dedupe_key", json!("")),
        ] {
            let mut bad = good.clone();
            bad[field] = value.clone();
            let problem = w.send(&plugin, &bad).await.problem(422);
            assert_eq!(problem.errors[0].field, field, "{field} = {value}");
        }
        // The target is not one of the two shapes, or both at once.
        for to in [
            json!({}),
            json!({"course_id": w.course}),
            json!({"user_id": w.s1.id, "course_id": w.course, "audience": "all"}),
            json!({"course_id": w.course, "audience": "everyone"}),
            json!("s1"),
        ] {
            let mut bad = good.clone();
            bad["to"] = to.clone();
            assert_eq!(w.send(&plugin, &bad).await.status, 422, "{to}");
        }
        assert_eq!(w.count("pending").await, 0, "nothing was queued");

        // 404 and 403 for the target.
        let unknown = w.send(&plugin, &mail(to_user(Uuid::now_v7()))).await;
        assert_eq!(unknown.status, 404);
        let unknown = w
            .send(&plugin, &mail(to_course(Uuid::now_v7(), "all")))
            .await;
        assert_eq!(unknown.status, 404);
        assert_eq!(w.count("pending").await, 0);
    })
    .await;
}

#[tokio::test]
async fn a_plugin_reaches_only_the_courses_it_was_granted() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w
            .plugin(
                "only-a",
                &[Scope::MailSend],
                CourseScope::Only(vec![w.course]),
            )
            .await;

        // A person of the granted course, a person of the other course, and the other course.
        assert_eq!(w.send(&plugin, &mail(to_user(w.s1.id))).await.status, 202);
        assert_eq!(w.send(&plugin, &mail(to_user(w.sb.id))).await.status, 403);
        assert_eq!(
            w.send(&plugin, &mail(to_course(w.other_course, "all")))
                .await
                .status,
            403
        );
        // Someone on both rosters is reachable through the course the plugin may reach.
        assert_eq!(
            w.send(&plugin, &mail(to_user(w.teacher.id))).await.status,
            202
        );
        assert_eq!(w.count("pending").await, 2);
        let _ = (&w.assistant, &w.s2);
    })
    .await;
}

#[tokio::test]
async fn without_mail_configured_the_route_is_503_after_the_scope_check() {
    db_test(async |pool| {
        let mut settings = common::test_settings();
        settings.plugins.secret_key = Secret::new("0123456789abcdef0123456789abcdef");
        let w = world_with(pool, Mailbox::new(), settings).await;
        let plugin = w.mailer("p").await;
        let reader = w
            .plugin("reader", &[Scope::ReportsWrite], CourseScope::All)
            .await;

        let response = w.send(&plugin, &mail(to_user(w.s1.id))).await;

        let problem = response.problem(503);
        let detail = problem.detail.clone().unwrap_or_default();
        assert!(detail.contains("not configured"), "{problem:?}");
        assert_eq!(w.send(&reader, &mail(to_user(w.s1.id))).await.status, 403);
    })
    .await;
}

#[tokio::test]
async fn people_without_a_verified_address_or_with_a_disabled_account_are_counted_not_mailed() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w.mailer("p").await;
        sqlx::query!(
            "UPDATE auth.users SET email_verified_at = NULL WHERE id = $1",
            w.s1.id
        )
        .execute(&w.env.pool)
        .await
        .unwrap();
        // A disabled student leaves the roster (ADR-0037); a disabled assistant stays on it.
        sqlx::query!(
            "UPDATE auth.users SET status = 'disabled' WHERE id = $1",
            w.assistant.id
        )
        .execute(&w.env.pool)
        .await
        .unwrap();

        let response = w.send(&plugin, &mail(to_course(w.course, "all"))).await;

        assert_eq!(response.status, 202);
        let body = response.json();
        assert_eq!(
            (body["accepted"].as_u64(), body["skipped"].as_u64()),
            (Some(2), Some(2))
        );
        // opted_out, no_address, duplicate, disabled, rate_limited
        assert_eq!(reasons(&response), [0, 1, 0, 1, 0]);
        assert_eq!(w.count("pending").await, 2);
        let one = w.send(&plugin, &mail(to_user(w.s1.id))).await;
        assert_eq!(reasons(&one), [0, 1, 0, 0, 0]);
    })
    .await;
}

#[tokio::test]
async fn an_audience_over_the_limit_is_422_on_to_and_queues_nothing() {
    db_test(async |pool| {
        let w = world(pool, |mail| mail.max_audience = 2).await;
        let plugin = w.mailer("p").await;

        // The course has two students: exactly the limit.
        assert_eq!(
            w.send(&plugin, &mail(to_course(w.course, "students")))
                .await
                .status,
            202
        );
        // Three people with the assistant: over it, and a retry could never help.
        let response = w.send(&plugin, &mail(to_course(w.course, "all"))).await;

        let problem = response.problem(422);
        assert_eq!(problem.errors[0].field, "to");
        assert!(response.headers.get("retry-after").is_none());
        assert_eq!(w.count("pending").await, 2, "only the first call queued");
    })
    .await;
}

#[tokio::test]
async fn the_hourly_limit_counts_accepted_calls_and_answers_429_with_retry_after() {
    db_test(async |pool| {
        let w = world(pool, |mail| mail.plugin_hourly_calls = 2).await;
        let busy = w.mailer("busy").await;
        let quiet = w.mailer("quiet").await;
        for i in 0..2 {
            let mut body = mail(to_user(w.s1.id));
            body["kind"] = json!(format!("kind-{i}"));
            assert_eq!(w.send(&busy, &body).await.status, 202);
        }

        let response = w.send(&busy, &mail(to_user(w.s2.id))).await;

        let problem = response.problem(429);
        let wait: u64 = response.header("retry-after").parse().unwrap();
        assert!((3500..=3600).contains(&wait), "{wait}");
        assert!(!problem.detail.unwrap_or_default().contains("lms.test"));
        // Another plugin has its own allowance, and nothing from the refused call was queued.
        assert_eq!(w.send(&quiet, &mail(to_user(w.s2.id))).await.status, 202);
        assert_eq!(w.count("pending").await, 3);
        // A refused call is not audited.
        let audited = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM events.audit_log WHERE action = 'plugin.mail.send'"#
        )
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(audited, 3);
    })
    .await;
}

#[tokio::test]
async fn the_daily_limit_is_429_for_one_person_and_a_skip_inside_an_audience() {
    db_test(async |pool| {
        let w = world(pool, |mail| mail.recipient_daily = 1).await;
        let plugin = w.mailer("p").await;
        let mut first = mail(to_user(w.s1.id));
        first["kind"] = json!("one");
        assert_eq!(w.send(&plugin, &first).await.status, 202);

        let mut second = mail(to_user(w.s1.id));
        second["kind"] = json!("two");
        let single = w.send(&plugin, &second).await;
        single.problem(429);
        let wait: u64 = single.header("retry-after").parse().unwrap();
        assert!((86_000..=86_400).contains(&wait), "{wait}");

        second["to"] = to_course(w.course, "students");
        let audience = w.send(&plugin, &second).await;
        assert_eq!(audience.status, 202);
        assert_eq!(
            audience.json()["accepted"],
            1,
            "Omar-like second student still gets it"
        );
        assert_eq!(reasons(&audience), [0, 0, 0, 0, 1]);
    })
    .await;
}

#[tokio::test]
async fn a_dedupe_key_is_accepted_once_per_person() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w.mailer("p").await;
        let mut body = mail(to_user(w.s1.id));
        body["dedupe_key"] = json!("assignment-7");
        assert_eq!(w.send(&plugin, &body).await.json()["accepted"], 1);

        let again = w.send(&plugin, &body).await;
        assert_eq!(again.status, 202);
        assert_eq!(again.json()["accepted"], 0);
        assert_eq!(reasons(&again), [0, 0, 1, 0, 0]);

        body["to"] = to_course(w.course, "students");
        let audience = w.send(&plugin, &body).await;
        assert_eq!(
            audience.json()["accepted"],
            1,
            "only the second student is new"
        );
        assert_eq!(w.count("pending").await, 2);
    })
    .await;
}

fn token_in(message: &str) -> String {
    let start = message
        .find("/api/v1/mail/opt-out/")
        .expect("an opt-out link")
        + "/api/v1/mail/opt-out/".len();
    message[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect()
}

#[tokio::test]
async fn the_link_in_the_footer_turns_one_kind_of_one_plugin_off_and_the_answer_turns_it_on_again()
{
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w.mailer("deadline-alert").await;
        let other = w.mailer("other-plugin").await;
        w.send(&plugin, &mail(to_user(w.s1.id))).await;
        let messages = w.delivered().await;
        let token = token_in(&messages[0]);
        let http = common::Http::new(&w.env.base);

        // The one-click form of a mail client: a POST with a form body, no session, no header.
        let response = http
            .post(&format!("/mail/opt-out/{token}"))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(b"List-Unsubscribe=One-Click".to_vec())
            .send()
            .await;
        assert_eq!(
            response.status,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        assert!(response.header("content-type").starts_with("text/plain"));
        assert_eq!(response.header("cache-control"), "no-store");
        let text = String::from_utf8(response.body.clone()).unwrap();
        assert!(text.contains("deadline-alert"), "{text}");
        assert!(
            text.contains(&format!("/api/v1/mail/opt-in/{token}")),
            "{text}"
        );

        // A GET does the same (a mail scanner opens links), and doing it twice changes nothing.
        assert_eq!(
            http.get(&format!("/mail/opt-out/{token}"))
                .send()
                .await
                .status,
            200
        );

        // This kind from this plugin is off for this person; nothing else is.
        let off = w.send(&plugin, &mail(to_user(w.s1.id))).await;
        assert_eq!(reasons(&off), [1, 0, 0, 0, 0]);
        let mut other_kind = mail(to_user(w.s1.id));
        other_kind["kind"] = json!("reminder");
        assert_eq!(w.send(&plugin, &other_kind).await.json()["accepted"], 1);
        assert_eq!(
            w.send(&other, &mail(to_user(w.s1.id))).await.json()["accepted"],
            1
        );
        assert_eq!(
            w.send(&plugin, &mail(to_user(w.s2.id))).await.json()["accepted"],
            1
        );

        // A POST of the opt-in link brings it back.
        assert_eq!(
            http.post(&format!("/mail/opt-in/{token}"))
                .send()
                .await
                .status,
            200
        );
        assert_eq!(
            w.send(&plugin, &mail(to_user(w.s1.id))).await.json()["accepted"],
            1
        );
    })
    .await;
}

#[tokio::test]
async fn a_token_this_server_did_not_make_is_a_404_problem_and_changes_nothing() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w.mailer("p").await;
        w.send(&plugin, &mail(to_user(w.s1.id))).await;
        let token = token_in(&w.delivered().await[0]);
        let http = common::Http::new(&w.env.base);

        let tampered = format!("{}x", token);
        let (payload, _) = token.split_once('.').unwrap();
        for bad in [
            "junk".to_owned(),
            "a.b".to_owned(),
            tampered,
            format!("{payload}.AAAA"),
        ] {
            for request in [
                http.get(&format!("/mail/opt-out/{bad}")),
                http.post(&format!("/mail/opt-out/{bad}")),
                http.get(&format!("/mail/opt-in/{bad}")),
            ] {
                request.send().await.problem(404);
            }
        }
        let outs = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM mail.opt_outs"#)
            .fetch_one(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(outs, 0);
    })
    .await;
}

#[tokio::test]
async fn an_opt_out_made_after_queueing_stops_the_mail_before_it_is_sent() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w.mailer("p").await;
        w.send(&plugin, &mail(to_user(w.s1.id))).await;
        w.send(&plugin, &mail(to_user(w.s2.id))).await;
        // Make a token for s1 the way a first mail would have: the key exists after the first send.
        let mut conn = w.env.pool.acquire().await.unwrap();
        sqlx::query!(
            "INSERT INTO mail.opt_outs (user_id, plugin_id, kind) VALUES ($1, 'p', 'deadline_changed')",
            w.s1.id
        )
        .execute(&mut *conn)
        .await
        .unwrap();
        drop(conn);

        let messages = w.delivered().await;

        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("s2@lms.test"));
        assert_eq!(w.count("cancelled").await, 1);
    })
    .await;
}

#[tokio::test]
async fn disabling_or_removing_a_plugin_cancels_its_queued_mail_for_good() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let gone = w.mailer("gone").await;
        let paused = w.mailer("paused").await;
        let stays = w.mailer("stays").await;
        for plugin in [&gone, &paused, &stays] {
            w.send(plugin, &mail(to_user(w.s1.id))).await;
            // A different kind each time keeps the daily count in view, not the opt-out.
        }
        assert_eq!(w.count("pending").await, 3);

        let mut tx = w.env.pool.begin().await.unwrap();
        plugins::set_status(&mut tx, "paused", plugins::InstallStatus::Disabled)
            .await
            .unwrap();
        plugins::remove(&mut tx, "gone").await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(w.count("cancelled").await, 2);

        // Enabling it again does not bring the mail back.
        let mut conn = w.env.pool.acquire().await.unwrap();
        plugins::set_status(&mut conn, "paused", plugins::InstallStatus::Enabled)
            .await
            .unwrap();
        drop(conn);
        let messages = w.delivered().await;
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("X-Vumlo-Plugin: stays"));
        let erased = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM mail.messages
               WHERE state = 'cancelled' AND (to_address IS NOT NULL) AND erase_at <= now()"#
        )
        .fetch_one(&w.env.pool)
        .await
        .unwrap();
        assert_eq!(erased, 2, "their addresses are due to be erased at once");
    })
    .await;
}

#[tokio::test]
async fn an_accepted_call_is_audited_by_ids_and_numbers_never_by_text_or_address() {
    db_test(async |pool| {
        let w = world(pool, |_| {}).await;
        let plugin = w.mailer("deadline-alert").await;
        let mut body = mail(to_course(w.course, "students"));
        body["subject"] = json!("Secret subject Okafor");
        body["text"] = json!("Secret text for s1@lms.test");
        w.send(&plugin, &body).await;
        w.delivered().await;

        let row = sqlx::query!(
            r#"SELECT source, action, resource_type, resource_id, meta, actor_id
               FROM events.audit_log WHERE action = 'plugin.mail.send'"#
        )
        .fetch_one(&w.env.pool)
        .await
        .unwrap();

        assert_eq!(row.source, "plugin");
        assert_eq!(row.actor_id, None);
        assert_eq!(row.resource_type, "mail_batch");
        let batch = sqlx::query_scalar!("SELECT id FROM mail.batches")
            .fetch_one(&w.env.pool)
            .await
            .unwrap();
        assert_eq!(row.resource_id, batch);
        assert_eq!(row.meta["kind"], "deadline_changed");
        assert_eq!(row.meta["plugin_id"], "deadline-alert");
        assert_eq!(row.meta["to"]["course_id"], w.course.to_string());
        assert_eq!(row.meta["to"]["audience"], "students");
        assert_eq!(row.meta["accepted"], 2);
        assert_eq!(row.meta["skipped"]["opted_out"], 0);
        let text = row.meta.to_string();
        for leaked in ["Secret", "Okafor", "lms.test", "s1@"] {
            assert!(!text.contains(leaked), "{leaked}: {text}");
        }

        // The admin's audit report shows the row without choking on the new resource type.
        let admin = w.env.user("admin@lms.test", Role::Admin).await;
        let report = w
            .env
            .ok(
                "GET",
                &admin.token,
                "/admin/audit?action=plugin.mail.send",
                None,
                200,
            )
            .await;
        assert_eq!(report["items"][0]["resource_type"], "mail_batch");
        let shown = report.to_string();
        assert!(
            !shown.contains("Secret") && !shown.contains("s1@lms.test"),
            "{shown}"
        );
        let _ = &w.teacher;
    })
    .await;
}
