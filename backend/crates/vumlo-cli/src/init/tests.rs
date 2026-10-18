use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vumlo-init-{name}-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn it_writes_both_files_once_and_never_overwrites() {
    let dir = scratch("twice");
    run(&dir, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("vumlo.toml")).unwrap(),
        CONFIG
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".env.example")).unwrap(),
        ENV
    );

    std::fs::write(dir.join("vumlo.toml"), "mine").unwrap();
    run(&dir, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("vumlo.toml")).unwrap(),
        "mine"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_dry_run_writes_nothing() {
    let dir = scratch("dry");
    run(&dir, true).unwrap();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}
