use super::*;

#[test]
fn the_connection_becomes_libpq_variables_with_the_password_decoded() {
    let env = libpq_env("postgres://vumlo:p%40ss%3Aw%2Frd@db.example.edu:5433/lms?sslmode=require")
        .unwrap();
    let get = |name: &str| {
        env.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("PGHOST"), Some("db.example.edu"));
    assert_eq!(get("PGPORT"), Some("5433"));
    assert_eq!(get("PGUSER"), Some("vumlo"));
    assert_eq!(get("PGDATABASE"), Some("lms"));
    assert_eq!(get("PGSSLMODE"), Some("require"));
    assert_eq!(get("PGPASSWORD"), Some("p@ss:w/rd"));
}

#[test]
fn a_url_without_a_password_sets_none() {
    let env = libpq_env("postgres://vumlo@localhost/lms").unwrap();
    assert!(!env.iter().any(|(k, _)| *k == "PGPASSWORD"));
}

#[test]
fn something_that_is_not_a_postgres_url_is_refused() {
    assert!(libpq_env("mysql://u:p@h/db").is_err());
}
