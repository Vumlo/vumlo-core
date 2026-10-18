//! `vumlo-cli build web`: brand apply, then npm. npm is a script that records how it was called.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{Env, db_test};

/// A directory with an `npm` that appends its arguments, the working directory and the generated
/// directory it was told about to `calls.log`, and exits with `exit_code`.
fn fake_npm(dir: &Path, exit_code: i32) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let script = dir.join("npm");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && echo 10.0.0 && exit 0\n\
             echo \"$* | $(basename \"$(pwd)\") | $VUMLO_GENERATED_DIR\" >> \"{}/calls.log\"\n\
             exit {exit_code}\n",
            dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    format!("{}:/usr/bin:/bin", dir.display())
}

fn web_app(env: &Env) {
    std::fs::create_dir_all(env.path("web")).unwrap();
    std::fs::write(env.path("web/package.json"), "{}").unwrap();
}

fn calls(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn it_applies_the_branding_then_installs_and_builds_with_the_generated_directory() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        web_app(&env);
        let bin = env.path("bin");
        let path = fake_npm(&bin, 0);

        let out = env
            .run_with(
                &["build", "web"],
                None,
                &[("DATABASE_URL", env.url.as_str()), ("PATH", path.as_str())],
            )
            .await;

        assert!(out.ok, "{}", out.stderr);
        assert!(
            env.path(".vumlo/generated/client-config.json").exists(),
            "brand apply ran first"
        );
        let log = calls(&bin);
        assert_eq!(log.len(), 2, "{log:?}");
        let generated = std::path::absolute(env.path(".vumlo/generated")).unwrap();
        assert_eq!(log[0], format!("install | web | {}", generated.display()));
        assert_eq!(log[1], format!("run build | web | {}", generated.display()));
    })
    .await;
}

#[tokio::test]
async fn it_does_not_install_again_when_node_modules_exist() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        web_app(&env);
        std::fs::create_dir_all(env.path("web/node_modules")).unwrap();
        let bin = env.path("bin");
        let path = fake_npm(&bin, 0);

        let out = env
            .run_with(
                &["build", "web"],
                None,
                &[("DATABASE_URL", env.url.as_str()), ("PATH", path.as_str())],
            )
            .await;

        assert!(out.ok, "{}", out.stderr);
        assert_eq!(calls(&bin).len(), 1);
    })
    .await;
}

#[tokio::test]
async fn a_failing_build_fails_the_command() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        web_app(&env);
        std::fs::create_dir_all(env.path("web/node_modules")).unwrap();
        let path = fake_npm(&env.path("bin"), 3);

        let out = env
            .run_with(
                &["build", "web"],
                None,
                &[("DATABASE_URL", env.url.as_str()), ("PATH", path.as_str())],
            )
            .await;

        assert!(!out.ok);
        assert!(out.stderr.contains("npm run build"), "{}", out.stderr);
    })
    .await;
}

#[tokio::test]
async fn without_npm_it_says_so_before_writing_anything() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        web_app(&env);
        let empty = env.path("empty");
        std::fs::create_dir_all(&empty).unwrap();

        let out = env
            .run_with(
                &["build", "web"],
                None,
                &[
                    ("DATABASE_URL", env.url.as_str()),
                    ("PATH", empty.to_str().unwrap()),
                ],
            )
            .await;

        assert!(!out.ok);
        assert!(out.stderr.contains("npm was not found"), "{}", out.stderr);
        assert!(!env.path(".vumlo").exists());
    })
    .await;
}

#[tokio::test]
async fn it_needs_the_web_app_and_a_dry_run_does_nothing() {
    db_test(async |pool| {
        let env = Env::new(&pool);
        let missing = env.run(&["build", "web"]).await;
        assert!(!missing.ok);
        assert!(
            missing.stderr.contains("no package.json"),
            "{}",
            missing.stderr
        );

        web_app(&env);
        let bin = env.path("bin");
        let path = fake_npm(&bin, 0);
        let dry = env
            .run_with(
                &["build", "web", "--dry-run"],
                None,
                &[("DATABASE_URL", env.url.as_str()), ("PATH", path.as_str())],
            )
            .await;
        assert!(dry.ok, "{}", dry.stderr);
        assert!(
            dry.stdout.contains("would run  npm run build"),
            "{}",
            dry.stdout
        );
        assert!(calls(&bin).is_empty());
        assert!(!env.path(".vumlo").exists());
    })
    .await;
}
