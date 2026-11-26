//! ADR-0098, decision D4: items of an activity type nobody registered (a module that is switched
//! off) stay in the database and read as unavailable. The course tree around them still loads,
//! their body is never shown, nothing can be written to them, and the module's type cannot be
//! chosen for a new item. The registry is process-wide, so this binary installs the types of a
//! `dummy` module that is off; `module_activity_types.rs` is the same world with it on.
#![cfg(feature = "dummy")]

mod common;

use std::collections::BTreeMap;

use ::common::Role;
use common::{Account, Env, db_test, id_of, test_settings};
use serde_json::{Value, json};
use server::modules;
use uuid::Uuid;
use vumlo_config::{ModuleEntry, Settings, resolve_modules};

fn disabled_settings() -> Settings {
    let mut settings = test_settings();
    settings.modules = BTreeMap::from([("dummy".to_owned(), ModuleEntry::Flag(false))]);
    settings.module_state = resolve_modules(&settings.modules, &BTreeMap::new(), &modules::specs())
        .expect("the dummy module resolves");
    settings
}

struct World {
    env: Env,
    course: Uuid,
    owner: Account,
    student: Account,
    module: Uuid,
}

async fn world(pool: sqlx::PgPool) -> World {
    activity::ensure_installed(|| {
        modules::activity_types(&disabled_settings()).expect("the built-in types")
    });
    let env = Env::new(pool).await;
    let owner = env.user("owner@lms.test", Role::Teacher).await;
    let student = env.user("student@lms.test", Role::Student).await;
    let (course, module) = env.course(&owner, &[&student]).await;
    World {
        env,
        course,
        owner,
        student,
        module,
    }
}

impl World {
    /// A published item of `item_type`, written straight to the table: the way it was left when
    /// the module that made it was switched off. An assignment also gets its row.
    async fn stored(&self, title: &str, kind: &str, item_type: &str) -> Uuid {
        let id = Uuid::now_v7();
        let body = json!({"v": 1, "blocks": [{"id": "a", "type": "paragraph", "props": {},
            "content": [{"type": "text", "text": "secret-words-of-the-module"}]}], "attachments": []});
        sqlx::query(
            "INSERT INTO content.items (id, module_id, course_id, position, kind, type, title,
                                        body, status, published_title, published_body,
                                        last_published_at)
             VALUES ($1, $2, $3,
                     COALESCE((SELECT max(position) FROM content.items WHERE module_id = $2), 0) + 1,
                     $4, $5, $6, $7, 'published', $6, $7, now())",
        )
        .bind(id)
        .bind(self.module)
        .bind(self.course)
        .bind(kind)
        .bind(item_type)
        .bind(title)
        .bind(&body)
        .execute(&self.env.pool)
        .await
        .unwrap();
        if kind == "assignment" {
            sqlx::query(
                "INSERT INTO content.assignments (item_id, config, max_points)
                 VALUES ($1, '{\"secret\": \"config\"}', 10)",
            )
            .bind(id)
            .execute(&self.env.pool)
            .await
            .unwrap();
        }
        id
    }

    async fn get(&self, who: &Account, path: &str) -> Value {
        self.env.ok("GET", &who.token, path, None, 200).await
    }
}

#[tokio::test]
async fn items_of_an_unregistered_type_are_listed_as_unavailable_and_the_tree_still_loads() {
    db_test(async |pool| {
        let w = world(pool).await;
        let note = w.stored("From a module", "resource", "dummy_note").await;
        let gone = w
            .stored("Graded by a module", "assignment", "dummy_graded")
            .await;
        let normal = id_of(
            &w.env
                .ok(
                    "POST",
                    &w.owner.token,
                    &format!("/modules/{}/items", w.module),
                    Some(json!({"title": "A lecture"})),
                    201,
                )
                .await,
        );

        for who in [&w.owner, &w.student] {
            let tree = w.get(who, &format!("/courses/{}/content", w.course)).await;
            let items = tree["modules"][0]["items"].as_array().unwrap();
            let by = |id: Uuid| items.iter().find(|i| i["id"] == json!(id)).unwrap();
            assert_eq!(by(note)["unavailable"], true, "{tree}");
            assert_eq!(by(note)["type"], "dummy_note");
            assert_eq!(by(gone)["unavailable"], true);
            assert_eq!(by(gone)["kind"], "assignment");
            // A lecture of the core is as it was; the flag is not sent.
            if who.id == w.owner.id {
                assert!(by(normal).get("unavailable").is_none());
            }
        }
    })
    .await;
}

#[tokio::test]
async fn the_body_and_the_config_of_an_unavailable_item_are_never_shown() {
    db_test(async |pool| {
        let w = world(pool).await;
        let note = w.stored("From a module", "resource", "dummy_note").await;
        let gone = w
            .stored("Graded by a module", "assignment", "dummy_graded")
            .await;

        for who in [&w.owner, &w.student] {
            let item = w.get(who, &format!("/items/{note}")).await;
            assert_eq!(item["unavailable"], true);
            assert_eq!(item["body"], json!({}), "{item}");
            let graded = w.get(who, &format!("/items/{gone}")).await;
            assert_eq!(graded["unavailable"], true);
            assert_eq!(graded["body"], json!({}));
            assert_eq!(graded["assignment"]["config"], json!({}), "{graded}");
            assert_eq!(
                graded["assignment"]["max_points"], "10.00",
                "the points are still known"
            );
        }
        // Search matches the title, and never quotes or matches the body.
        let by_text = w.get(&w.student, "/search?q=secret-words").await;
        assert_eq!(by_text["items"], json!([]), "{by_text}");
        let by_title = w.get(&w.student, "/search?q=From%20a%20module").await;
        assert_eq!(by_title["items"].as_array().unwrap().len(), 1, "{by_title}");
        assert!(
            by_title["items"][0]
                .get("snippet")
                .is_none_or(Value::is_null)
        );
    })
    .await;
}

#[tokio::test]
async fn nothing_can_be_written_to_an_unavailable_item_and_its_type_cannot_be_chosen() {
    db_test(async |pool| {
        let w = world(pool).await;
        let note = w.stored("From a module", "resource", "dummy_note").await;

        let edit = w
            .env
            .call(
                "PATCH",
                &w.owner.token,
                &format!("/items/{note}"),
                Some(json!({"body": {"v": 1, "blocks": [], "attachments": []}})),
            )
            .await;
        assert_eq!(edit.status, 422, "{}", String::from_utf8_lossy(&edit.body));
        // A student cannot mark what they cannot see.
        let mark = w
            .env
            .call(
                "PUT",
                &w.student.token,
                &format!("/items/{note}/completion"),
                None,
            )
            .await;
        assert_eq!(mark.status, 409);
        // The type of the module that is off is not one a new item can have.
        let create = w
            .env
            .call(
                "POST",
                &w.owner.token,
                &format!("/modules/{}/items", w.module),
                Some(json!({"type": "dummy_note", "title": "Again"})),
            )
            .await;
        assert_eq!(create.status, 422);
    })
    .await;
}
