use serde_json::Value;

use super::*;
use crate::schema;
use crate::tests::{VALID, build};

fn leaves(value: &Value, prefix: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaves(child, &path, out);
            }
        }
        _ => out.push(prefix.to_owned()),
    }
}

#[test]
fn the_public_part_is_exactly_the_keys_marked_public() {
    let settings = build(VALID, &[]).unwrap();
    let client = ClientConfig::new(&settings, "1.2.3", None);
    let mut shown = Vec::new();
    leaves(
        &serde_json::to_value(&client.public).unwrap(),
        "",
        &mut shown,
    );
    shown.sort();
    let mut marked: Vec<String> = schema::public_paths()
        .into_iter()
        .map(str::to_owned)
        .collect();
    marked.sort();
    assert_eq!(shown, marked);
}

#[test]
fn private_and_secret_values_never_reach_the_client_config() {
    let toml = format!(
        "{VALID}\n[auth]\nallowed_domains = [\"sentinel-domain.edu\"]\n\
             [mail]\nfrom = \"sentinel-from@x.edu\"\n[mail.transport]\nkind = \"smtp\"\n\
             host = \"sentinel-host.edu\"\n\
             [modules.dummy]\nenabled = true\nlabel = \"sentinel-label\"\n\
             endpoint = \"https://sentinel-endpoint.edu\"\n"
    );
    let env = [
        (
            "DATABASE_URL",
            "postgres://sentinel-db-user:sentinel-db-pass@h/db",
        ),
        ("VUMLO__MAIL__TRANSPORT__PASSWORD", "sentinel-smtp-password"),
        ("VUMLO_DUMMY_API_KEY", "sentinel-module-key"),
        ("VUMLO__SERVER__CLIENT_IP_HEADER", "sentinel-header"),
    ];
    let settings = crate::tests::build_with(&toml, &env, &["dummy"]).unwrap();
    let client = ClientConfig::new(&settings, "1", Some("logo.png".to_owned()));

    let text = serde_json::to_string(&client).unwrap();

    assert!(!text.contains("sentinel"), "{text}");
    assert!(text.contains("\"dummy\":\"enabled\""), "{text}");
    assert!(text.contains("https://lms.example.edu/api/v1"), "{text}");
    assert_eq!(client.web_modules, ["dummy"]);
}

#[test]
fn only_enabled_modules_with_a_web_part_are_built_into_the_web_app() {
    let off = build(VALID, &[]).unwrap();
    assert!(ClientConfig::new(&off, "1", None).web_modules.is_empty());
    let on = crate::tests::build_with(
        &format!("{VALID}\n[modules]\ndummy = true\n"),
        &[],
        &["dummy"],
    )
    .unwrap();
    assert_eq!(ClientConfig::new(&on, "1", None).web_modules, ["dummy"]);
}

#[test]
fn every_module_of_the_registry_is_listed_with_its_state() {
    let settings = build(VALID, &[]).unwrap();
    let client = ClientConfig::new(&settings, "1", None);
    let names: Vec<_> = client.modules.keys().map(String::as_str).collect();
    let known: Vec<_> = crate::modules::known_modules()
        .iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(names, known);
    assert!(
        client
            .modules
            .values()
            .all(|s| *s == ModuleStatus::Unavailable)
    );
}

// --- the scheme of the desktop app's links (ADR-0062) --------------------------------------------

fn scheme_of(extra: &str) -> String {
    let settings = build(&format!("{VALID}\n{extra}"), &[]).unwrap();
    ClientConfig::new(&settings, "1.2.3", None)
        .desktop
        .url_scheme
}

#[test]
fn the_scheme_is_vumlo_and_the_short_name_unless_the_config_names_one() {
    assert_eq!(
        scheme_of(""),
        "vumlo-example-university",
        "no short name: the name"
    );
    assert_eq!(
        scheme_of("[desktop]\nurl_scheme = \"vumlo-ucu\""),
        "vumlo-ucu"
    );
}

#[test]
fn a_short_name_with_no_ascii_letters_leaves_plain_vumlo() {
    let toml = VALID.replace(
        "name = \"Example University\"",
        "name = \"Університет\"\nshort_name = \"УКУ\"",
    );
    let settings = build(&toml, &[]).unwrap();
    assert_eq!(url_scheme(&settings), "vumlo");
    let toml = VALID.replace(
        "name = \"Example University\"",
        "name = \"X\"\nshort_name = \"  U.C.U  (Kyiv) \"",
    );
    assert_eq!(url_scheme(&build(&toml, &[]).unwrap()), "vumlo-u-c-u-kyiv");
}

#[test]
fn a_scheme_that_is_not_ours_is_refused_with_its_key_named() {
    for bad in [
        "ucu",
        "vumlo-",
        "vumlo-UCU",
        "vumlo-u cu",
        "vumlo--ucu",
        "vumlo-ucu-",
        "vumlo-ucu://",
        "http",
    ] {
        let toml = format!("{VALID}\n[desktop]\nurl_scheme = \"{bad}\"");
        let error = build(&toml, &[]).unwrap_err().to_string();
        assert!(
            error.contains("desktop.url_scheme") && error.contains(bad),
            "{bad}: {error}"
        );
    }
    assert!(
        build(&format!("{VALID}\n[desktop]\nbogus = 1"), &[]).is_err(),
        "a stray key is refused"
    );
}

#[test]
fn the_scheme_is_in_the_client_config() {
    let settings = build(VALID, &[]).unwrap();
    let json = serde_json::to_value(ClientConfig::new(&settings, "1.2.3", None)).unwrap();
    assert_eq!(json["desktop"]["url_scheme"], "vumlo-example-university");
}
