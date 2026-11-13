//! `vumlo-cli roles ...` (ADR-0094): what the unit roles `manager` and `viewer` may do. A change is
//! one audit row with the source `cli`; the same set again changes nothing and writes nothing.

mod common;

use common::{Env, Out, audit, audit_meta, db_test};
use serde_json::json;

/// The line of `roles show` that starts with `role`.
fn line_of(out: &Out, role: &str) -> String {
    out.stdout
        .lines()
        .find(|line| line.starts_with(role))
        .unwrap_or_else(|| panic!("no {role} line in:\n{}", out.stdout))
        .to_owned()
}

async fn show(env: &Env) -> Out {
    let out = env.run(&["roles", "show"]).await;
    assert!(out.ok, "{}", out.stderr);
    out
}

#[tokio::test]
async fn show_lists_the_switches_of_each_role_and_says_viewing_is_always_on() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);

        let out = show(&env).await;

        assert!(out.stdout.contains("always"), "{}", out.stdout);
        assert!(
            line_of(&out, "manager")
                .contains("view_grades, manage_roster, manage_courses, manage_units"),
            "{}",
            out.stdout
        );
        assert!(
            line_of(&out, "viewer").ends_with("view_grades"),
            "{}",
            out.stdout
        );
        assert!(audit(&pool).await.is_empty(), "show writes nothing");
    })
    .await;
}

#[tokio::test]
async fn set_replaces_the_switches_with_one_audit_row_and_the_same_set_changes_nothing() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);

        // Given out of order and twice, stored in order.
        let set = env
            .run(&[
                "roles",
                "set",
                "viewer",
                "--abilities",
                "manage_roster,view_grades,manage_roster",
            ])
            .await;
        assert!(set.ok, "{}", set.stderr);
        assert!(
            line_of(&show(&env).await, "viewer").ends_with("view_grades, manage_roster"),
            "{}",
            set.stdout
        );
        assert_eq!(
            audit(&pool).await,
            [(
                "unit.abilities.set".to_owned(),
                "cli".to_owned(),
                true,
                "unit_role".to_owned()
            )]
        );
        assert_eq!(
            audit_meta(&pool, "unit.abilities.set").await,
            json!({"role": "viewer", "abilities": ["view_grades", "manage_roster"]})
        );

        let again = env
            .run(&[
                "roles",
                "set",
                "viewer",
                "--abilities",
                "view_grades,manage_roster",
            ])
            .await;
        assert!(
            again.ok && again.stdout.contains("nothing was changed"),
            "{}{}",
            again.stdout,
            again.stderr
        );
        assert_eq!(audit(&pool).await.len(), 1, "the repeat writes nothing");
        assert!(
            line_of(&show(&env).await, "manager").contains("manage_units"),
            "the other role is not touched"
        );
    })
    .await;
}

#[tokio::test]
async fn an_unknown_ability_or_role_is_refused_with_the_valid_names_and_changes_nothing() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);

        let bad = env
            .run(&["roles", "set", "viewer", "--abilities", "view_grades,bogus"])
            .await;
        assert!(!bad.ok, "{}", bad.stdout);
        for name in ["bogus", "view_grades", "edit_content", "manage_units"] {
            assert!(bad.stderr.contains(name), "{name} in:\n{}", bad.stderr);
        }
        let role = env
            .run(&["roles", "set", "owner", "--abilities", "view_grades"])
            .await;
        assert!(
            !role.ok && role.stderr.contains("manager") && role.stderr.contains("viewer"),
            "{}",
            role.stderr
        );

        assert!(line_of(&show(&env).await, "viewer").ends_with("view_grades"));
        assert!(audit(&pool).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn none_takes_every_switch_away_and_a_dry_run_changes_nothing() {
    db_test(async |pool| {
        common::migrate_modules(&pool).await;
        let env = Env::new(&pool);

        let dry = env
            .run(&["roles", "set", "viewer", "--abilities", "none", "--dry-run"])
            .await;
        assert!(
            dry.ok && dry.stdout.contains("would set"),
            "{}{}",
            dry.stdout,
            dry.stderr
        );
        assert!(line_of(&show(&env).await, "viewer").ends_with("view_grades"));
        assert!(audit(&pool).await.is_empty());

        let none = env
            .run(&["roles", "set", "viewer", "--abilities", "none"])
            .await;
        assert!(none.ok, "{}", none.stderr);
        assert!(line_of(&show(&env).await, "viewer").contains("view only"));
        assert_eq!(
            audit_meta(&pool, "unit.abilities.set").await,
            json!({"role": "viewer", "abilities": []})
        );
    })
    .await;
}
