//! `vumlo-cli brand apply`: the generated directory the web build reads.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{CONFIG, Env, db_test};
use serde_json::Value;

const LOGO: &[u8] = include_bytes!("../../../../brand/logo.png");

/// Every file under `dir`, by name.
fn tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

fn dimensions(png: &[u8]) -> (u32, u32) {
    let decoded = image::load_from_memory(png).expect("a PNG");
    (decoded.width(), decoded.height())
}

#[tokio::test]
async fn it_writes_the_config_the_types_the_schema_the_manifest_and_the_vumlo_icons() {
    db_test(async |pool| {
        let env = Env::new(&pool);

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let files = tree(&env.path(".vumlo/generated"));
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "apple-touch-icon.png",
                "client-config.d.ts",
                "client-config.json",
                "favicon.ico",
                "icon-1024.png",
                "icon-192.png",
                "icon-512.png",
                "logo.png",
                "site.webmanifest",
                "vumlo.schema.json"
            ]
        );
        assert_eq!(
            files["logo.png"], LOGO,
            "the logo is Vumlo's, copied as it is"
        );
        assert_eq!(dimensions(&files["icon-512.png"]), (512, 512));
        assert_eq!(dimensions(&files["icon-192.png"]), (192, 192));
        assert_eq!(
            dimensions(&files["icon-1024.png"]),
            (1024, 1024),
            "what the installers of the desktop app are made from"
        );
        assert_eq!(dimensions(&files["apple-touch-icon.png"]), (180, 180));
        // An ICO file: reserved 0, type 1, three images.
        assert_eq!(&files["favicon.ico"][..6], &[0, 0, 1, 0, 3, 0]);
        let client: Value = serde_json::from_slice(&files["client-config.json"]).unwrap();
        assert!(client.get("logo").is_none());
        assert_eq!(client["public"]["institution"]["name"], "Test University");
        let manifest: Value = serde_json::from_slice(&files["site.webmanifest"]).unwrap();
        assert_eq!(manifest["icons"][1]["src"], "/brand/icon-512.png");
        assert_eq!(manifest["name"], "Vumlo | Test University");
        assert_eq!(manifest["short_name"], "Vumlo");
        assert!(
            String::from_utf8_lossy(&files["client-config.d.ts"])
                .contains("export type ClientConfig")
        );
    })
    .await;
}

#[tokio::test]
async fn an_institution_logo_in_the_config_is_refused_and_the_error_says_it_is_gone() {
    db_test(async |pool| {
        let env = Env::with_config(
            &pool,
            &CONFIG.replace(
                "mode = \"university\"",
                "mode = \"university\"\nlogo = \"logo.png\"",
            ),
        );

        let out = env.run(&["brand", "apply"]).await;

        assert!(!out.ok);
        assert!(out.stderr.contains("institution.logo"), "{}", out.stderr);
        assert!(!env.path(".vumlo/generated").exists());
    })
    .await;
}

#[tokio::test]
async fn brand_apply_is_idempotent_and_starts_from_scratch() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        assert!(env.run(&["brand", "apply"]).await.ok);
        let first = tree(&env.path(".vumlo/generated"));

        std::fs::write(env.path(".vumlo/generated/stale.txt"), "left over").unwrap();
        assert!(env.run(&["brand", "apply"]).await.ok);
        let second = tree(&env.path(".vumlo/generated"));

        assert_eq!(
            first, second,
            "the same input gives the same bytes, and the stale file is gone"
        );
        assert!(!env.path(".vumlo/generated.tmp").exists());
    })
    .await;
}

/// What `init` writes: an installation with the example configuration builds.
#[tokio::test]
async fn the_example_configuration_builds() {
    const EXAMPLE: &str = include_str!("../../../../deploy/vumlo.example.toml");
    db_test(async |pool| {
        let env = Env::with_config(&pool, EXAMPLE);

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let files = tree(&env.path(".vumlo/generated"));
        let client: Value = serde_json::from_slice(&files["client-config.json"]).unwrap();
        assert_eq!(client["public"]["institution"]["name"], "Vumlo");
        let manifest: Value = serde_json::from_slice(&files["site.webmanifest"]).unwrap();
        assert_eq!(manifest["name"], "Vumlo", "no \"Vumlo | Vumlo\"");
        assert!(files.contains_key("favicon.ico") && files.contains_key("icon-512.png"));
    })
    .await;
}

#[tokio::test]
async fn a_dry_run_writes_nothing() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let out = env.run(&["brand", "apply", "--dry-run"]).await;
        assert!(
            out.ok && out.stdout.contains("would write  client-config.json"),
            "{}",
            out.stderr
        );
        assert!(!env.path(".vumlo").exists());
    })
    .await;
}

#[tokio::test]
async fn private_and_secret_keys_never_reach_the_generated_files() {
    db_test(async |pool| {
        let config = format!(
            "{CONFIG}\n[auth]\nallowed_domains = [\"sentinel-domain.edu\"]\n\
             [mail]\nfrom = \"sentinel-from@x.edu\"\n[mail.transport]\nkind = \"smtp\"\n\
             host = \"sentinel-host.edu\"\n[cli]\ncore_url = \"http://sentinel-core.edu:8080\"\n"
        );
        let env = Env::with_config(&pool, &config);
        let vars = [
            (
                "DATABASE_URL",
                "postgres://sentinel-user:sentinel-password@127.0.0.1:1/db",
            ),
            ("VUMLO__MAIL__TRANSPORT__PASSWORD", "sentinel-smtp"),
            ("VUMLO__SERVER__CLIENT_IP_HEADER", "sentinel-header"),
        ];

        let out = env.run_with(&["brand", "apply"], None, &vars).await;

        assert!(out.ok, "{}", out.stderr);
        for (name, bytes) in tree(&env.path(".vumlo/generated")) {
            // The schema describes keys and may name them; it holds no value of this installation.
            // The icons are images.
            if name.ends_with(".png") || name.ends_with(".ico") {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                !text.contains("sentinel"),
                "{name} leaks a private value: {text}"
            );
        }
        let client: Value = serde_json::from_slice(
            &std::fs::read(env.path(".vumlo/generated/client-config.json")).unwrap(),
        )
        .unwrap();
        assert!(client["public"]["institution"].get("mode").is_none());
        assert!(client.get("database").is_none() && client.get("auth").is_none());
    })
    .await;
}
