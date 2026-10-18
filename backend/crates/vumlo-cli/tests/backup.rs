//! `vumlo-cli db backup`, with scripts standing in for pg_dump and pg_restore.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::{CONFIG, Env};

const URL: &str = "postgres://backup_user:pw%40x%3Ay@dbhost:5544/lmsdb";

/// A directory with a `pg_dump` and a `pg_restore` that log how they were called. `dump_exit` is
/// the exit code of the fake pg_dump; `restore_exit` that of `pg_restore --list`.
fn fake_tools(dir: &Path, dump_exit: i32, restore_exit: i32) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let log = dir.join("calls.log");
    let write = |name: &str, body: String| {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    write(
        "pg_dump",
        format!(
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && echo pg_dump 18 && exit 0\n\
             echo \"pg_dump $*\" >> \"{log}\"\n\
             echo \"env $PGHOST $PGPORT $PGUSER $PGDATABASE $PGPASSWORD $PGSSLMODE\" >> \"{log}\"\n\
             while [ $# -gt 0 ]; do [ \"$1\" = \"--file\" ] && out=\"$2\"; shift; done\n\
             echo DUMP > \"$out\"\nexit {dump_exit}\n",
            log = log.display()
        ),
    );
    write(
        "pg_restore",
        format!(
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] && echo pg_restore 18 && exit 0\n\
             echo \"pg_restore $*\" >> \"{log}\"\nexit {restore_exit}\n",
            log = log.display()
        ),
    );
    format!("{}:/usr/bin:/bin", dir.display())
}

fn log(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("calls.log")).unwrap_or_default()
}

async fn backup(env: &Env, path: &str, args: &[&str]) -> common::Out {
    let mut all = vec!["db", "backup"];
    all.extend_from_slice(args);
    env.run_with(&all, None, &[("DATABASE_URL", URL), ("PATH", path)])
        .await
}

#[tokio::test]
async fn it_dumps_with_the_connection_in_the_environment_and_reads_the_dump_back() {
    let env = Env::at(URL.to_owned(), CONFIG);
    let tools = env.path("tools");
    let path = fake_tools(&tools, 0, 0);
    let target = env.path("out/vumlo.dump");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();

    let out = backup(&env, &path, &["--output", target.to_str().unwrap()]).await;

    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("backup:") && out.stdout.contains("readable by pg_restore"));
    assert_eq!(std::fs::read_to_string(&target).unwrap().trim(), "DUMP");
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let calls = log(&tools);
    assert!(
        calls.contains("env dbhost 5544 backup_user lmsdb pw@x:y require")
            || calls.contains("env dbhost 5544 backup_user lmsdb pw@x:y prefer"),
        "{calls}"
    );
    assert!(calls.contains("pg_dump --format=custom --file"), "{calls}");
    assert!(
        calls.contains(&format!("pg_restore --list {}", target.display())),
        "{calls}"
    );
    for line in calls.lines().filter(|l| l.starts_with("pg_")) {
        assert!(
            !line.contains("pw") && !line.contains("backup_user"),
            "an argument leaks the login: {line}"
        );
    }
}

#[tokio::test]
async fn it_never_overwrites_a_backup() {
    let env = Env::at(URL.to_owned(), CONFIG);
    let path = fake_tools(&env.path("tools"), 0, 0);
    let target = env.path("vumlo.dump");
    std::fs::write(&target, "precious").unwrap();

    let out = backup(&env, &path, &["--output", target.to_str().unwrap()]).await;

    assert!(!out.ok);
    assert!(out.stderr.contains("must not exist"), "{}", out.stderr);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "precious");
}

#[tokio::test]
async fn a_failing_dump_leaves_no_file() {
    let env = Env::at(URL.to_owned(), CONFIG);
    let path = fake_tools(&env.path("tools"), 1, 0);
    let target = env.path("vumlo.dump");

    let out = backup(&env, &path, &["--output", target.to_str().unwrap()]).await;

    assert!(!out.ok);
    assert!(out.stderr.contains("pg_dump failed"), "{}", out.stderr);
    assert!(!target.exists());
}

#[tokio::test]
async fn a_dump_that_does_not_read_back_is_removed() {
    let env = Env::at(URL.to_owned(), CONFIG);
    let path = fake_tools(&env.path("tools"), 0, 1);
    let target = env.path("vumlo.dump");

    let out = backup(&env, &path, &["--output", target.to_str().unwrap()]).await;

    assert!(!out.ok);
    assert!(out.stderr.contains("does not read back"), "{}", out.stderr);
    assert!(!target.exists());
}

#[tokio::test]
async fn without_the_client_tools_it_says_which_package_to_install() {
    let env = Env::at(URL.to_owned(), CONFIG);
    let empty = env.path("empty");
    std::fs::create_dir_all(&empty).unwrap();

    let out = backup(&env, empty.to_str().unwrap(), &[]).await;

    assert!(!out.ok);
    assert!(out.stderr.contains("postgresql-client"), "{}", out.stderr);
}

#[tokio::test]
async fn by_default_the_file_is_named_after_the_time_and_lands_in_the_current_directory() {
    let env = Env::at(URL.to_owned(), CONFIG);
    let path = fake_tools(&env.path("tools"), 0, 0);

    let out = backup(&env, &path, &[]).await;

    assert!(out.ok, "{}", out.stderr);
    let dumps: Vec<_> = std::fs::read_dir(&env.dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("vumlo-") && n.ends_with(".dump"))
        .collect();
    assert_eq!(dumps.len(), 1, "{dumps:?}");
}
