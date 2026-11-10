//! `vumlo-cli brand apply`: the files the web build reads, written straight into `web/`.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{CONFIG, Env, db_test};
use serde_json::Value;

const LOGO: &[u8] = include_bytes!("../../../../brand/vumlo-mark.png");

/// A web app to brand: `brand apply` has nothing to do without a `package.json` in `web/`.
fn web_app(env: &Env) {
    std::fs::create_dir_all(env.path("web")).unwrap();
    std::fs::write(env.path("web/package.json"), "{}").unwrap();
}

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
async fn it_writes_the_config_and_the_types_into_src_and_the_manifest_and_icons_into_public() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        web_app(&env);

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        assert!(!env.path(".vumlo").exists());
        let client = tree(&env.path("web/src/generated"));
        assert_eq!(
            client.keys().map(String::as_str).collect::<Vec<_>>(),
            ["client-config.d.ts", "client-config.json"]
        );
        let files = tree(&env.path("web/public/brand"));
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "apple-touch-icon.png",
                "favicon.ico",
                "icon-1024.png",
                "icon-192.png",
                "icon-512.png",
                "site.webmanifest",
            ]
        );
        let icon = image::load_from_memory(&files["icon-1024.png"])
            .unwrap()
            .to_rgba8();
        let mark = image::load_from_memory(LOGO).unwrap().to_rgba8();
        assert_eq!(
            icon.get_pixel(512, 512).0,
            image::imageops::resize(&mark, 1024, 1024, image::imageops::FilterType::Lanczos3)
                .get_pixel(512, 512)
                .0,
            "the icons are made from the Vumlo mark"
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
        let config: Value = serde_json::from_slice(&client["client-config.json"]).unwrap();
        assert!(config.get("logo").is_none());
        assert_eq!(config["public"]["institution"]["name"], "Test University");
        assert_eq!(
            config["public"]["institution"]["accent"], "violet",
            "the accent reaches the browser, violet by default"
        );
        let manifest: Value = serde_json::from_slice(&files["site.webmanifest"]).unwrap();
        assert_eq!(manifest["icons"][1]["src"], "/brand/icon-512.png");
        assert_eq!(manifest["name"], "Vumlo | Test University");
        assert_eq!(manifest["short_name"], "Vumlo");
        assert!(
            String::from_utf8_lossy(&client["client-config.d.ts"])
                .contains("export type ClientConfig")
        );
    })
    .await;
}

#[tokio::test]
async fn the_accent_of_the_configuration_reaches_the_client_config() {
    db_test(async |pool| {
        let env = Env::with_config(
            &pool,
            &CONFIG.replace(
                "mode = \"university\"",
                "mode = \"university\"\naccent = \"teal\"",
            ),
        );
        web_app(&env);

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let config: Value = serde_json::from_slice(
            &std::fs::read(env.path("web/src/generated/client-config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["public"]["institution"]["accent"], "teal");
    })
    .await;
}

#[tokio::test]
async fn without_a_web_app_there_is_nothing_to_do() {
    db_test(async |pool| {
        let env = Env::new(&pool);

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        assert!(out.stdout.contains("nothing to do"), "{}", out.stdout);
        assert!(!env.path("web").exists() && !env.path(".vumlo").exists());
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
        web_app(&env);

        let out = env.run(&["brand", "apply"]).await;

        assert!(!out.ok);
        assert!(out.stderr.contains("institution.logo"), "{}", out.stderr);
        assert!(!env.path("web/src/generated").exists());
        assert!(!env.path("web/public/brand").exists());
    })
    .await;
}

#[tokio::test]
async fn brand_apply_is_idempotent_and_starts_from_scratch() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        web_app(&env);
        assert!(env.run(&["brand", "apply"]).await.ok);
        let first = (
            tree(&env.path("web/src/generated")),
            tree(&env.path("web/public/brand")),
        );

        std::fs::write(env.path("web/src/generated/stale.txt"), "left over").unwrap();
        std::fs::write(env.path("web/public/brand/stale.txt"), "left over").unwrap();
        assert!(env.run(&["brand", "apply"]).await.ok);
        let second = (
            tree(&env.path("web/src/generated")),
            tree(&env.path("web/public/brand")),
        );

        assert_eq!(
            first, second,
            "the same input gives the same bytes, and the stale files are gone"
        );
        assert!(!env.path("web/src/generated.tmp").exists());
        assert!(!env.path("web/public/brand.tmp").exists());
    })
    .await;
}

/// What `init` writes: an installation with the example configuration builds.
#[tokio::test]
async fn the_example_configuration_builds() {
    const EXAMPLE: &str = include_str!("../../../../deploy/vumlo.example.toml");
    db_test(async |pool| {
        let env = Env::with_config(&pool, EXAMPLE);
        web_app(&env);

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let client: Value = serde_json::from_slice(
            &std::fs::read(env.path("web/src/generated/client-config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(client["public"]["institution"]["name"], "Vumlo");
        let files = tree(&env.path("web/public/brand"));
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
        web_app(&env);
        let out = env.run(&["brand", "apply", "--dry-run"]).await;
        assert!(
            out.ok && out.stdout.contains("would write  client-config.json"),
            "{}",
            out.stderr
        );
        assert!(!env.path("web/src").exists() && !env.path("web/public").exists());
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
        web_app(&env);
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
        let written = tree(&env.path("web/src/generated"))
            .into_iter()
            .chain(tree(&env.path("web/public/brand")));
        for (name, bytes) in written {
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
            &std::fs::read(env.path("web/src/generated/client-config.json")).unwrap(),
        )
        .unwrap();
        assert!(client["public"]["institution"].get("mode").is_none());
        assert!(client.get("database").is_none() && client.get("auth").is_none());
    })
    .await;
}
