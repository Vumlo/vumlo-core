use super::*;

fn entries(toml: &[(&str, ModuleEntry)]) -> BTreeMap<String, ModuleEntry> {
    toml.iter()
        .map(|(k, v)| ((*k).to_owned(), v.clone()))
        .collect()
}

fn table(pairs: &[(&str, serde_json::Value)]) -> ModuleEntry {
    ModuleEntry::Table(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    )
}

fn no_env() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[test]
fn an_unknown_module_is_refused_and_the_compiled_ones_are_listed() {
    let e = resolve(
        &entries(&[("ai-agent", ModuleEntry::Flag(true))]),
        &no_env(),
        &test_specs(&["dummy"]),
    )
    .unwrap_err();
    assert!(
        e[0].contains("modules.ai-agent") && e[0].contains("this build has: dummy"),
        "{e:?}"
    );
}

#[test]
fn a_module_is_unavailable_disabled_or_enabled() {
    let off = resolve(
        &entries(&[("dummy", ModuleEntry::Flag(false))]),
        &no_env(),
        &test_specs(&["dummy"]),
    )
    .unwrap();
    assert_eq!(off.status("dummy"), ModuleStatus::Disabled);
    let on = resolve(
        &entries(&[("dummy", ModuleEntry::Flag(true))]),
        &no_env(),
        &test_specs(&["dummy"]),
    )
    .unwrap();
    assert_eq!(on.status("dummy"), ModuleStatus::Enabled);
    let none = resolve(&BTreeMap::new(), &no_env(), &[]).unwrap();
    assert_eq!(none.status("dummy"), ModuleStatus::Unavailable);
    assert!(none.statuses().is_empty());
    let one = resolve(&BTreeMap::new(), &no_env(), &test_specs(&["dummy"])).unwrap();
    assert_eq!(one.statuses().len(), 1);
}

#[test]
fn a_module_that_is_not_compiled_in_is_refused_on_or_off() {
    for flag in [true, false] {
        let e = resolve(
            &entries(&[("dummy", ModuleEntry::Flag(flag))]),
            &no_env(),
            &[],
        )
        .unwrap_err();
        assert!(
            e[0].contains("unknown, or not compiled into this binary"),
            "{e:?}"
        );
    }
}

#[test]
fn a_module_whose_dependency_is_off_cannot_be_enabled() {
    let e = resolve(
        &entries(&[("dummy-dep", ModuleEntry::Flag(true))]),
        &no_env(),
        &test_specs(&["dummy", "dummy-dep"]),
    )
    .unwrap_err();
    assert!(e.iter().any(|m| m.contains("needs modules.dummy")), "{e:?}");
    resolve(
        &entries(&[
            ("dummy", ModuleEntry::Flag(true)),
            ("dummy-dep", ModuleEntry::Flag(true)),
        ]),
        &no_env(),
        &test_specs(&["dummy", "dummy-dep"]),
    )
    .unwrap();
}

#[test]
fn module_settings_are_checked_and_a_secret_in_the_file_is_refused() {
    let e = resolve(
        &entries(&[(
            "dummy",
            table(&[
                ("enabled", true.into()),
                ("api_key", "hunter2".into()),
                ("nope", 1.into()),
            ]),
        )]),
        &no_env(),
        &test_specs(&["dummy"]),
    )
    .unwrap_err();
    assert!(e.iter().any(|m| m.contains("VUMLO_DUMMY_API_KEY")), "{e:?}");
    assert!(!e.iter().any(|m| m.contains("hunter2")), "{e:?}");
    assert!(
        e.iter()
            .any(|m| m.contains("modules.dummy.nope: unknown setting")),
        "{e:?}"
    );
}

#[test]
fn a_module_secret_comes_from_its_environment_variable() {
    let env = BTreeMap::from([("VUMLO_DUMMY_API_KEY".to_owned(), "k".to_owned())]);
    let m = resolve(
        &entries(&[("dummy", table(&[("enabled", true.into())]))]),
        &env,
        &test_specs(&["dummy"]),
    )
    .unwrap();
    assert_eq!(m.get("dummy").unwrap().secrets["api_key"].expose(), "k");
    assert!(!format!("{:?}", m.get("dummy").unwrap()).contains("\"k\""));
}

#[test]
fn endpoints_need_https_and_no_credentials() {
    assert!(check_endpoint("https://api.example.com/v1", false).is_ok());
    for bad in [
        "http://api.example.com",
        "ftp://x",
        "api.example.com",
        "https://user:pw@api.example.com",
        "https://",
        "https://api.example.com/v1?key=1",
    ] {
        assert!(check_endpoint(bad, false).is_err(), "{bad}");
        assert!(check_endpoint(bad, true).is_err(), "{bad} with the flag");
    }
    assert!(check_endpoint("http://localhost:8080", false).is_err());
    assert!(check_endpoint("http://localhost:8080", true).is_ok());
    assert!(check_endpoint("http://127.0.0.1:1/x", true).is_ok());
    assert!(check_endpoint("http://[::1]:1/x", true).is_ok());
    assert!(check_endpoint("http://example.com", true).is_err());
}

#[test]
fn an_endpoint_setting_is_checked_when_the_table_is_read() {
    let e = resolve(
        &entries(&[("dummy", table(&[("endpoint", "http://example.com".into())]))]),
        &no_env(),
        &test_specs(&["dummy"]),
    )
    .unwrap_err();
    assert!(e[0].contains("modules.dummy.endpoint"), "{e:?}");
}
