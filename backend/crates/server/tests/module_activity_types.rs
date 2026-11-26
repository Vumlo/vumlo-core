//! ADR-0098: a module adds an activity type. With the module enabled its type is registered with
//! the built-in ones, so an item of it can be created, published and marked done like a lecture.
//! The registry is process-wide, so this binary installs the types of an enabled `dummy` module
//! before any server starts; `module_activity_types_off.rs` is the same world with it off.
#![cfg(feature = "dummy")]

mod common;

use std::collections::BTreeMap;

use ::common::Role;
use common::{Account, Env, db_test, id_of, test_settings};
use serde_json::{Value, json};
use server::modules;
use uuid::Uuid;
use vumlo_config::{ModuleEntry, Settings, resolve_modules};

fn enabled_settings() -> Settings {
    let mut settings = test_settings();
    settings.modules = BTreeMap::from([("dummy".to_owned(), ModuleEntry::Flag(true))]);
    settings.module_state = resolve_modules(&settings.modules, &BTreeMap::new(), &modules::specs())
        .expect("the dummy module resolves");
    settings
}

struct World {
    env: Env,
    owner: Account,
    student: Account,
    module: Uuid,
}

async fn world(pool: sqlx::PgPool) -> World {
    activity::ensure_installed(|| {
        modules::activity_types(&enabled_settings()).expect("the built-in types and dummy_note")
    });
    let env = Env::new(pool).await;
    let owner = env.user("owner@lms.test", Role::Teacher).await;
    let student = env.user("student@lms.test", Role::Student).await;
    let (_course, module) = env.course(&owner, &[&student]).await;
    World {
        env,
        owner,
        student,
        module,
    }
}

#[tokio::test]
async fn an_enabled_module_type_is_created_published_and_marked_done_like_a_lecture() {
    db_test(async |pool| {
        let w = world(pool).await;
        let note: Value = w
            .env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/modules/{}/items", w.module),
                Some(json!({"type": "dummy_note", "title": "A note"})),
                201,
            )
            .await;
        assert_eq!(note["type"], "dummy_note");
        assert_eq!(note["kind"], "resource");
        assert!(note.get("unavailable").is_none(), "{note}");
        assert!(note.get("assignment").is_none());
        let id = id_of(&note);

        w.env
            .ok(
                "POST",
                &w.owner.token,
                &format!("/items/{id}/publish"),
                None,
                200,
            )
            .await;
        // Its completion policy is `Marked`: the student marks it, and the mark is kept.
        w.env
            .ok(
                "PUT",
                &w.student.token,
                &format!("/items/{id}/completion"),
                None,
                204,
            )
            .await;

        // A type nobody registered is refused when creating, as before.
        let refused = w
            .env
            .call(
                "POST",
                &w.owner.token,
                &format!("/modules/{}/items", w.module),
                Some(json!({"type": "nobody_made_this", "title": "x"})),
            )
            .await;
        assert_eq!(refused.status, 422);
    })
    .await;
}
