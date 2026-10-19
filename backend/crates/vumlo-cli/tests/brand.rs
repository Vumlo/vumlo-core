//! `vumlo-cli brand apply`: the generated directory the web build reads.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{CONFIG, Env, db_test};
use serde_json::Value;

/// A 120 x 60 PNG: not square, to see the margin.
fn png_logo() -> Vec<u8> {
    use image::ImageEncoder;
    let image = image::RgbaImage::from_pixel(120, 60, image::Rgba([200, 30, 30, 255]));
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(image.as_raw(), 120, 60, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

const SVG_LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100">
  <rect width="200" height="100" fill="#1b4f9c"/><circle cx="100" cy="50" r="30" fill="#ffffff"/></svg>"##;

fn with_logo(extra: &str) -> String {
    CONFIG.replace(
        "mode = \"university\"",
        &format!("mode = \"university\"\n{extra}"),
    )
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
async fn without_a_logo_it_writes_the_config_the_types_the_schema_and_the_manifest() {
    db_test(async |pool| {
        let env = Env::new(&pool);

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let files = tree(&env.path(".vumlo/generated"));
        assert_eq!(
            files.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "client-config.d.ts",
                "client-config.json",
                "site.webmanifest",
                "vumlo.schema.json"
            ]
        );
        let client: Value = serde_json::from_slice(&files["client-config.json"]).unwrap();
        assert_eq!(client["logo"], Value::Null);
        assert_eq!(client["public"]["institution"]["name"], "Test University");
        let manifest: Value = serde_json::from_slice(&files["site.webmanifest"]).unwrap();
        assert_eq!(manifest["icons"], serde_json::json!([]));
        assert!(
            String::from_utf8_lossy(&files["client-config.d.ts"])
                .contains("export type ClientConfig")
        );
    })
    .await;
}

#[tokio::test]
async fn a_png_logo_gives_the_favicon_and_the_icons() {
    db_test(async |pool| {
        let env = Env::with_config(&pool, &with_logo("logo = \"logo.png\""));
        std::fs::write(env.path("logo.png"), png_logo()).unwrap();

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let files = tree(&env.path(".vumlo/generated"));
        assert_eq!(dimensions(&files["icon-512.png"]), (512, 512));
        assert_eq!(dimensions(&files["icon-192.png"]), (192, 192));
        assert_eq!(
            dimensions(&files["icon-1024.png"]),
            (1024, 1024),
            "what the installers of the desktop app are made from"
        );
        assert_eq!(dimensions(&files["apple-touch-icon.png"]), (180, 180));
        assert_eq!(files["logo.png"], png_logo(), "the logo is copied as it is");
        // An ICO file: reserved 0, type 1, three images.
        assert_eq!(&files["favicon.ico"][..6], &[0, 0, 1, 0, 3, 0]);
        let manifest: Value = serde_json::from_slice(&files["site.webmanifest"]).unwrap();
        assert_eq!(manifest["icons"][1]["src"], "/brand/icon-512.png");
        let client: Value = serde_json::from_slice(&files["client-config.json"]).unwrap();
        assert_eq!(client["logo"], "logo.png");
    })
    .await;
}

#[tokio::test]
async fn an_svg_logo_is_rendered_in_rust() {
    db_test(async |pool| {
        let env = Env::with_config(&pool, &with_logo("logo = \"brand/logo.svg\""));
        std::fs::create_dir_all(env.path("brand")).unwrap();
        std::fs::write(env.path("brand/logo.svg"), SVG_LOGO).unwrap();

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let files = tree(&env.path(".vumlo/generated"));
        assert_eq!(files["logo.svg"], SVG_LOGO.as_bytes());
        let icon = image::load_from_memory(&files["icon-512.png"])
            .unwrap()
            .to_rgba8();
        assert_eq!(icon.dimensions(), (512, 512));
        // The 2:1 logo is centred with a transparent margin above and below, the blue in the middle.
        assert_eq!(icon.get_pixel(256, 5).0[3], 0);
        let middle = icon.get_pixel(20, 256).0;
        assert!(middle[2] > middle[0], "blue, not red: {middle:?}");
        assert_eq!(middle[3], 255);
    })
    .await;
}

#[tokio::test]
async fn a_logo_can_be_a_url() {
    db_test(async |pool| {
        let logo = png_logo();
        let app = axum::Router::new().route(
            "/logo.png",
            axum::routing::get(move || {
                let logo = logo.clone();
                async move { ([("content-type", "image/png")], logo) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await });
        let env = Env::with_config(
            &pool,
            &with_logo(&format!("logo = \"http://{addr}/logo.png\"")),
        );

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        assert!(env.path(".vumlo/generated/icon-512.png").exists());
    })
    .await;
}

#[tokio::test]
async fn a_logo_that_is_not_an_image_or_is_missing_is_refused() {
    db_test(async |pool| {
        let env = Env::with_config(&pool, &with_logo("logo = \"logo.gif\""));
        let missing = env.run(&["brand", "apply"]).await;
        assert!(!missing.ok);
        let check = env.run(&["config", "check"]).await;
        assert!(
            !check.ok && check.stderr.contains("institution.logo"),
            "{}",
            check.stderr
        );

        std::fs::write(env.path("logo.gif"), b"GIF89a....").unwrap();
        let gif = env.run(&["brand", "apply"]).await;
        assert!(!gif.ok);
        assert!(
            gif.stderr.contains("not a PNG, JPEG, WebP or SVG"),
            "{}",
            gif.stderr
        );
        assert!(
            !env.path(".vumlo/generated").exists(),
            "a failed run writes nothing"
        );
    })
    .await;
}

#[tokio::test]
async fn brand_apply_is_idempotent_and_starts_from_scratch() {
    db_test(async |pool| {
        let env = Env::with_config(&pool, &with_logo("logo = \"logo.png\""));
        std::fs::write(env.path("logo.png"), png_logo()).unwrap();
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

/// What `init` writes and the placeholder logo of the repository: an installation with no assets
/// of its own builds, and the institution's own logo replaces every trace of the placeholder.
#[tokio::test]
async fn the_placeholder_brand_builds_and_is_replaced_completely() {
    const EXAMPLE: &str = include_str!("../../../../deploy/vumlo.example.toml");
    const PLACEHOLDER: &[u8] = include_bytes!("../../../../brand/logo.png");
    db_test(async |pool| {
        let env = Env::with_config(&pool, EXAMPLE);
        std::fs::create_dir_all(env.path("brand")).unwrap();
        std::fs::write(env.path("brand/logo.png"), PLACEHOLDER).unwrap();

        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let files = tree(&env.path(".vumlo/generated"));
        let client: Value = serde_json::from_slice(&files["client-config.json"]).unwrap();
        assert_eq!(client["public"]["institution"]["name"], "Vumlo");
        assert_eq!(client["logo"], "logo.png");
        assert_eq!(files["logo.png"], PLACEHOLDER);
        assert!(files.contains_key("favicon.ico") && files.contains_key("icon-512.png"));

        std::fs::write(env.path("logo.png"), png_logo()).unwrap();
        let own = EXAMPLE.replace("logo = \"brand/logo.png\"", "logo = \"logo.png\"");
        std::fs::write(env.path("vumlo.toml"), own).unwrap();
        let out = env.run(&["brand", "apply"]).await;

        assert!(out.ok, "{}", out.stderr);
        let files = tree(&env.path(".vumlo/generated"));
        assert_eq!(files["logo.png"], png_logo(), "the placeholder is gone");
        let icon = image::load_from_memory(&files["icon-512.png"])
            .unwrap()
            .to_rgba8();
        let middle = icon.get_pixel(256, 256).0;
        assert!(
            middle[0] > middle[2],
            "red, not the placeholder: {middle:?}"
        );
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
            "{}\n[auth]\nallowed_domains = [\"sentinel-domain.edu\"]\n\
             [mail]\nfrom = \"sentinel-from@x.edu\"\n[mail.transport]\nkind = \"smtp\"\n\
             host = \"sentinel-host.edu\"\n[cli]\ncore_url = \"http://sentinel-core.edu:8080\"\n",
            with_logo("logo = \"sentinel-logo.png\"")
        );
        let env = Env::with_config(&pool, &config);
        std::fs::write(env.path("sentinel-logo.png"), png_logo()).unwrap();
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
            // The logo is the institution's own file, copied on purpose.
            if name.ends_with(".png") || name.ends_with(".ico") || name.starts_with("logo.") {
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
