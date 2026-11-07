//! ADR-0070 in the CLI: what an administrator writes for the Buddy is checked when the configuration
//! is read, the module migrates into its own schema, and `doctor` says what is wrong or kept.
#![cfg(feature = "ai_agent")]

mod common;

use common::{CONFIG, Env, audit_meta, bare_db_test};
use serde_json::{Value, json};

const GOOD: &str = "endpoint = \"https://api.example.invalid/v1\"\nmodel = \"m\"\n";

fn buddy(settings: &str) -> String {
    format!("{CONFIG}\n[modules.ai_agent]\nenabled = true\n{settings}")
}

fn find(report: &Value, name: &str) -> Value {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no {name} in {report:#}"))
}

#[tokio::test]
async fn the_endpoint_and_the_model_are_required_only_while_the_buddy_is_on() {
    bare_db_test(async |pool| {
        let on = Env::with_config(&pool, &buddy(""))
            .run(&["config", "check"])
            .await;
        assert!(!on.ok);
        assert!(
            on.stderr.contains("modules.ai_agent.endpoint is required"),
            "{}",
            on.stderr
        );
        assert!(
            on.stderr.contains("modules.ai_agent.model is required"),
            "{}",
            on.stderr
        );

        let off = format!("{CONFIG}\n[modules.ai_agent]\nenabled = false\n");
        let out = Env::with_config(&pool, &off)
            .run(&["config", "check"])
            .await;
        assert!(out.ok, "{}", out.stderr);
    })
    .await;
}

#[tokio::test]
async fn limits_are_whole_numbers_and_the_key_is_never_read_from_the_file() {
    bare_db_test(async |pool| {
        let bad = buddy(&format!(
            "{GOOD}daily_cap = \"ten\"\nretention_days = -1\napi_key = \"in-the-file\"\n"
        ));
        let out = Env::with_config(&pool, &bad).run(&["config", "check"]).await;
        assert!(!out.ok);
        for expected in [
            "modules.ai_agent.daily_cap must be a whole number",
            "modules.ai_agent.retention_days must be a whole number",
            "VUMLO_AI_AGENT_API_KEY",
        ] {
            assert!(out.stderr.contains(expected), "{expected}: {}", out.stderr);
        }
        assert!(!out.stderr.contains("in-the-file"));

        let good = buddy(&format!(
            "{GOOD}daily_cap = 20\nretention_days = 7\nroles = \"student,teacher\"\nnotice = \"Hi\"\n"
        ));
        let out = Env::with_config(&pool, &good).run(&["config", "check"]).await;
        assert!(out.ok, "{}", out.stderr);
    })
    .await;
}

#[tokio::test]
async fn migrate_creates_the_buddys_own_schema_and_modules_list_reports_it_on() {
    bare_db_test(async |pool| {
        let env = Env::with_config(&pool, &buddy(GOOD));
        let out = env.run(&["db", "migrate"]).await;
        assert!(out.ok, "{}", out.stderr);
        assert_eq!(
            audit_meta(&pool, "db.migrate").await["applied"]["ai_agent"],
            json!([1, 2])
        );
        let tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_tables
             WHERE schemaname = 'mod_ai_agent' AND tablename IN ('messages', 'usage', 'proposals')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(tables, 3);

        let listed: Value =
            serde_json::from_str(&env.run(&["modules", "list", "--json"]).await.stdout).unwrap();
        assert_eq!(listed["optional"]["ai_agent"], "enabled");
    })
    .await;
}

#[tokio::test]
async fn doctor_warns_about_a_misspelt_role_an_unreachable_model_and_chats_kept_while_off() {
    bare_db_test(async |pool| {
        let on = Env::with_config(
            &pool,
            &buddy(&format!("{GOOD}roles = \"student,techer\"\n")),
        );
        assert!(on.run(&["db", "migrate"]).await.ok);

        let report: Value =
            serde_json::from_str(&on.run(&["doctor", "--json"]).await.stdout).unwrap();
        let roles = find(&report, "ai_agent roles");
        assert_eq!(roles["status"], "warn");
        assert!(
            roles["detail"].as_str().unwrap().contains("techer"),
            "{roles}"
        );
        // Reachability only; the host does not exist, so this is a warning and not a failure.
        assert_eq!(find(&report, "ai_agent model endpoint")["status"], "warn");

        sqlx::query(
            "INSERT INTO mod_ai_agent.messages (id, user_id, role, content)
             VALUES (gen_random_uuid(), gen_random_uuid(), 'user', 'x')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let off = Env::with_config(
            &pool,
            &format!("{CONFIG}\n[modules.ai_agent]\nenabled = false\n"),
        );
        let report: Value =
            serde_json::from_str(&off.run(&["doctor", "--json"]).await.stdout).unwrap();
        let data = find(&report, "ai_agent data");
        assert_eq!(data["status"], "warn");
        assert!(
            data["detail"].as_str().unwrap().contains("not deleted"),
            "{data}"
        );
    })
    .await;
}
