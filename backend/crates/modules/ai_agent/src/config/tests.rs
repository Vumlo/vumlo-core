use std::collections::BTreeMap;

use serde_json::{Value, json};
use vumlo_config::{ResolvedModule, Secret};

use super::*;

fn resolved(pairs: &[(&str, Value)]) -> ResolvedModule {
    ResolvedModule {
        enabled: true,
        settings: pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
        ..ResolvedModule::default()
    }
}

#[test]
fn defaults_apply_when_nothing_is_set() {
    let r = resolved(&[]);
    let c = Config::new(&r);
    assert_eq!(c.daily_cap(), 100);
    assert_eq!(c.retention_days(), 30);
    assert!(c.allows(Persona::Teacher) && c.allows(Persona::Admin));
    assert!(
        !c.allows(Persona::Student),
        "students are off until an admin turns them on"
    );
    assert_eq!(c.notice(), None);
    assert_eq!(c.api_key(), None);
}

#[test]
fn roles_are_trimmed_lowercased_and_unknown_tokens_match_nobody() {
    let r = resolved(&[("roles", json!(" Student , ADMIN,, techer "))]);
    let c = Config::new(&r);
    assert!(c.allows(Persona::Student) && c.allows(Persona::Admin));
    assert!(
        !c.allows(Persona::Teacher),
        "a misspelt token enables nothing"
    );
    assert_eq!(c.role_tokens(), ["student", "admin", "techer"]);
}

#[test]
fn numbers_and_notice_come_from_the_file_and_retention_is_at_least_a_day() {
    let r = resolved(&[
        ("daily_cap", json!(0)),
        ("retention_days", json!(0)),
        ("notice", json!("  ")),
    ]);
    let c = Config::new(&r);
    assert_eq!(c.daily_cap(), 0, "0 is no limit");
    assert_eq!(c.retention_days(), 1);
    assert_eq!(c.notice(), None, "a blank notice is no notice");
}

#[test]
fn the_api_key_is_read_from_the_secrets_not_the_settings() {
    let r = ResolvedModule {
        secrets: BTreeMap::from([("api_key".to_owned(), Secret::new("k"))]),
        ..resolved(&[])
    };
    assert_eq!(Config::new(&r).api_key(), Some("k"));
}
