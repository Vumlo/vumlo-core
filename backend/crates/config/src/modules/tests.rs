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
fn an_int_setting_takes_a_whole_number_and_nothing_else() {
    const SPEC: &[ModuleSpec] = &[ModuleSpec {
        name: "counted",
        requires: &[],
        web: false,
        settings: &[SettingSpec {
            key: "cap",
            kind: SettingKind::Int,
            required: false,
        }],
    }];
    let with = |v: serde_json::Value| {
        resolve(
            &entries(&[("counted", table(&[("enabled", true.into()), ("cap", v)]))]),
            &no_env(),
            SPEC,
        )
    };
    assert_eq!(
        with(50.into()).unwrap().get("counted").unwrap().settings["cap"],
        50
    );
    for bad in [(-1).into(), 1.5.into(), "50".into(), true.into()] {
        let e = with(bad).unwrap_err();
        assert!(
            e.iter()
                .any(|m| m.contains("modules.counted.cap must be a whole number")),
            "{e:?}"
        );
    }
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
    assert!(
        check_endpoint("http://plugin-text-metrics:8000", true).is_err(),
        "the general endpoint check does not take a container name"
    );
}

#[test]
fn a_plugin_may_be_reached_over_http_by_the_name_of_a_container() {
    for ok in [
        "https://plugin.example.com",
        "http://localhost:8000",
        "http://[::1]:8000",
        "http://plugin-text-metrics:8000",
        "http://plugin_text:8000/base",
        "http://core",
    ] {
        assert!(check_plugin_endpoint(ok).is_ok(), "{ok}");
    }
    for bad in [
        "http://plugin.example.com:8000",
        "http://plugin-text-metrics.:8000",
        "http://10.0.0.5:8000",
        "http://127:8000",
        "http://2130706433",
        "http://0x7f.1",
        "http://[fe80::1]:8000",
        "http://-x:8000",
        "http://plug in:8000",
        "http://user@plugin-text-metrics:8000",
        "http://plugin-text-metrics:8000/x?k=1",
        "ftp://plugin-text-metrics",
    ] {
        assert!(check_plugin_endpoint(bad).is_err(), "{bad}");
    }
    let too_long = format!("http://a{}", "b".repeat(63));
    assert!(check_plugin_endpoint(&too_long).is_err());
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
