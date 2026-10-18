use std::collections::BTreeMap;

use super::*;
use crate::load::check_renamed;

pub(crate) const VALID: &str = r#"
    [server]
    bind_addr = "127.0.0.1:8080"
    public_origin = "https://lms.example.edu/"
    [database]
    max_connections = 5
    [log]
    format = "json"
    [institution]
    name = "Example University"
    mode = "university"
"#;

const DATABASE_URL: &str = "postgres://u:p@localhost/db";

/// The file text and the environment variables `env`; `DATABASE_URL` is set unless `env` sets it.
pub(crate) fn build_with(
    toml: &str,
    env: &[(&str, &str)],
    compiled: &[&str],
) -> Result<Settings, Error> {
    let mut vars: BTreeMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    if !vars.contains_key("DATABASE_URL") && !vars.contains_key("VUMLO__DATABASE__URL") {
        vars.insert("DATABASE_URL".to_owned(), DATABASE_URL.to_owned());
    }
    load(
        Sources {
            file: Some(FileSource::Text(toml.to_owned())),
            env: vars,
            overrides: Vec::new(),
        },
        compiled,
    )
}

pub(crate) fn build(toml: &str, env: &[(&str, &str)]) -> Result<Settings, Error> {
    build_with(toml, env, &[])
}

fn build_set(toml: &str, env: &[(&str, &str)], set: &[&str]) -> Result<Settings, Error> {
    let env: BTreeMap<String, String> = env
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .chain([("DATABASE_URL".to_owned(), DATABASE_URL.to_owned())])
        .collect();
    let set: Vec<String> = set.iter().map(|s| (*s).to_owned()).collect();
    load(
        Sources {
            file: Some(FileSource::Text(toml.to_owned())),
            env,
            overrides: parse_overrides(&set)?,
        },
        &[],
    )
}

fn names(vars: &[&str]) -> Vec<String> {
    vars.iter().map(|v| (*v).to_owned()).collect()
}

// --- the Learnly rename -------------------------------------------------------------------

#[test]
fn the_old_config_file_is_refused_unless_the_new_one_exists() {
    let e = check_renamed(true, false, &[]).unwrap_err().to_string();
    assert!(e.contains("rename it to vumlo.toml"), "{e}");
    assert!(check_renamed(true, true, &[]).is_ok());
    assert!(check_renamed(false, false, &[]).is_ok());
}

#[test]
fn an_old_variable_is_refused_and_names_its_replacement() {
    let vars = names(&["LEARNLY__DATABASE__URL", "LEARNLY_API_ORIGIN", "PATH"]);
    let e = check_renamed(false, false, &vars).unwrap_err().to_string();
    assert!(e.contains("rename it to VUMLO__DATABASE__URL"), "{e}");
    assert!(e.contains("rename it to VUMLO_API_ORIGIN"), "{e}");
    let both = names(&["LEARNLY__DATABASE__URL", "VUMLO__DATABASE__URL"]);
    assert!(check_renamed(false, false, &both).is_ok());
    assert!(check_renamed(false, false, &names(&["VUMLO__LOG__FORMAT"])).is_ok());
}

#[test]
fn an_old_variable_in_the_environment_stops_the_load() {
    let e = build(VALID, &[("LEARNLY__LOG__FORMAT", "json")]).unwrap_err();
    assert!(
        e.to_string().contains("rename it to VUMLO__LOG__FORMAT"),
        "{e}"
    );
}

// --- precedence ---------------------------------------------------------------------------

#[test]
fn defaults_apply_when_nothing_else_says_otherwise() {
    let toml = "[server]\npublic_origin = \"https://a.test\"\n[institution]\nname = \"A\"\nmode = \"school\"";
    let s = build(toml, &[]).unwrap();
    assert_eq!(s.server.bind_addr.to_string(), "127.0.0.1:8080");
    assert_eq!(s.database.max_connections, 10);
    assert_eq!(s.log.format, LogFormat::Pretty);
    assert_eq!(s.web.dir, std::path::Path::new("web"));
    assert_eq!(s.cli.env_file, std::path::Path::new(".env"));
    assert_eq!(s.config_version, SUPPORTED_CONFIG_VERSION);
}

#[test]
fn each_layer_overrides_the_one_before_it() {
    // default 10 < file 5 < environment 7 < flag 9
    assert_eq!(build(VALID, &[]).unwrap().database.max_connections, 5);
    let env = [("VUMLO__DATABASE__MAX_CONNECTIONS", "7")];
    assert_eq!(build(VALID, &env).unwrap().database.max_connections, 7);
    let set = ["database.max_connections=9"];
    assert_eq!(
        build_set(VALID, &env, &set)
            .unwrap()
            .database
            .max_connections,
        9
    );
    // and a flag beats the file when the environment is silent
    assert_eq!(
        build_set(VALID, &[], &set)
            .unwrap()
            .database
            .max_connections,
        9
    );
}

#[test]
fn a_flag_can_set_a_list_a_flag_and_a_name() {
    let s = build_set(
        VALID,
        &[],
        &[
            "auth.allowed_domains=a.edu, b.edu",
            "auth.cookie_secure=false",
            "institution.name=Flagged",
        ],
    )
    .unwrap();
    assert_eq!(s.auth.allowed_domains, ["a.edu", "b.edu"]);
    assert!(!s.auth.cookie_secure);
    assert_eq!(s.institution.name, "Flagged");
    assert!(build_set(VALID, &[], &["no-equals-sign"]).is_err());
}

#[test]
fn only_double_underscore_variables_are_configuration() {
    // VUMLO_API_ORIGIN belongs to the web app; it must not become a config key.
    let s = build(
        VALID,
        &[
            ("VUMLO_API_ORIGIN", "http://x"),
            ("VUMLO_FEATURES", "dummy"),
        ],
    );
    assert!(s.is_ok(), "{s:?}");
}

// --- secrets ------------------------------------------------------------------------------

#[test]
fn the_database_url_comes_from_database_url_and_not_from_the_file() {
    let s = build(VALID, &[("DATABASE_URL", "postgres://env/db")]).unwrap();
    assert_eq!(s.database.url.expose(), "postgres://env/db");
    let s = build(
        VALID,
        &[
            ("DATABASE_URL", "postgres://plain/db"),
            ("VUMLO__DATABASE__URL", "postgres://prefixed/db"),
        ],
    )
    .unwrap();
    assert_eq!(s.database.url.expose(), "postgres://prefixed/db");

    let toml = VALID.replace(
        "max_connections = 5",
        "max_connections = 5\nurl = \"postgres://file/db\"",
    );
    let e = build(&toml, &[]).unwrap_err().to_string();
    assert!(
        e.contains("database.url is a secret") && e.contains("DATABASE_URL"),
        "{e}"
    );
    assert!(
        !e.contains("postgres://file"),
        "the value is not echoed: {e}"
    );
}

#[test]
fn without_a_database_url_the_load_says_which_variable_to_set() {
    let e = load(
        Sources {
            file: Some(FileSource::Text(VALID.to_owned())),
            env: BTreeMap::new(),
            overrides: Vec::new(),
        },
        &[],
    )
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("database.url") && e.contains("DATABASE_URL"),
        "{e}"
    );
}

#[test]
fn every_secret_key_is_refused_in_the_file_and_names_its_variable() {
    let google = format!(
        "{VALID}\n[auth.google]\nclient_id = \"id\"\nclient_secret = \"s\"\nredirect_uri = \"https://x\"\n"
    );
    let e = build(&google, &[]).unwrap_err().to_string();
    assert!(e.contains("VUMLO__AUTH__GOOGLE__CLIENT_SECRET"), "{e}");
    let smtp = format!(
        "{VALID}\n[mail]\nfrom = \"a@b.edu\"\n[mail.transport]\nkind = \"smtp\"\nhost = \"h\"\npassword = \"p\"\n"
    );
    let e = build(&smtp, &[]).unwrap_err().to_string();
    assert!(e.contains("VUMLO__MAIL__TRANSPORT__PASSWORD"), "{e}");
}

#[test]
fn an_unknown_key_that_looks_like_a_secret_is_refused_as_one() {
    let toml = VALID.replace(
        "max_connections = 5",
        "max_connections = 5\nsession_signing_key = \"abc\"",
    );
    let e = build(&toml, &[]).unwrap_err().to_string();
    assert!(e.contains("looks like a secret"), "{e}");
}

#[test]
fn a_secret_cannot_be_set_with_a_flag() {
    let e = build_set(VALID, &[], &["database.url=postgres://flag/db"])
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("database.url is a secret") && e.contains("--set"),
        "{e}"
    );
    assert!(!e.contains("postgres://flag"), "{e}");
}

#[test]
fn a_secret_from_the_environment_is_kept_and_never_shown() {
    let toml = format!(
        "{VALID}\n[mail]\nfrom = \"a@b.edu\"\n[mail.transport]\nkind = \"smtp\"\nhost = \"mail.example.edu\"\ntls = \"tls\"\nusername = \"vumlo\"\n"
    );
    let s = build(
        &toml,
        &[(
            "VUMLO__MAIL__TRANSPORT__PASSWORD",
            "hunter2 is not a secret here",
        )],
    )
    .unwrap();
    let MailTransport::Smtp {
        host,
        port,
        tls,
        password,
        ..
    } = &s.mail.as_ref().unwrap().transport
    else {
        panic!("an smtp transport");
    };
    assert_eq!(
        (host.as_str(), *port, *tls),
        ("mail.example.edu", None, SmtpTls::Tls)
    );
    assert_eq!(
        password.as_ref().unwrap().expose(),
        "hunter2 is not a secret here"
    );
    for shown in [format!("{s:?}"), serde_json::to_string(&s).unwrap()] {
        assert!(
            !shown.contains("hunter2") && !shown.contains("u:p@localhost"),
            "{shown}"
        );
    }
}

// --- unknown keys, version ----------------------------------------------------------------

#[test]
fn an_unknown_key_is_an_error_that_names_it() {
    let e = build(VALID, &[("VUMLO__SERVER__PUBLIC_URL", "https://x.test")])
        .unwrap_err()
        .to_string();
    assert!(e.contains("server.public_url: unknown key"), "{e}");
}

#[test]
fn the_accent_is_gone_and_the_error_says_so() {
    let toml = VALID.replace(
        "mode = \"university\"",
        "mode = \"university\"\naccent = \"teal\"",
    );
    let e = build(&toml, &[]).unwrap_err().to_string();
    assert!(e.contains("institution.accent"), "{e}");
}

#[test]
fn a_config_version_newer_than_the_binary_is_refused() {
    let e = build(&format!("config_version = 2\n{VALID}"), &[])
        .unwrap_err()
        .to_string();
    assert!(e.contains("config_version 2"), "{e}");
    assert!(build(&format!("config_version = 1\n{VALID}"), &[]).is_ok());
}

#[test]
fn all_the_problems_are_reported_together() {
    let toml = VALID
        .replace("https://lms.example.edu/", "lms.example.edu")
        .replace("mode = \"university\"", "mode = \"college\"");
    let e = build(&toml, &[("VUMLO__AUTH__PASSWORD_MIN_LENGTH", "3")]).unwrap_err();
    assert!(e.to_string().contains("institution.mode"), "{e}");
    let e = build(
        &VALID.replace("https://lms.example.edu/", "lms.example.edu"),
        &[("VUMLO__AUTH__PASSWORD_MIN_LENGTH", "3")],
    )
    .unwrap_err();
    assert_eq!(e.0.len(), 2, "{e:?}");
}

// --- modules ------------------------------------------------------------------------------

#[test]
fn modules_are_read_from_the_file_and_the_environment() {
    let on = format!("{VALID}\n[modules]\ndummy = true\n");
    let s = build_with(&on, &[], &["dummy"]).unwrap();
    assert_eq!(s.module_state.status("dummy"), ModuleStatus::Enabled);

    let s = build_with(VALID, &[("VUMLO__MODULES__DUMMY", "true")], &["dummy"]).unwrap();
    assert_eq!(s.module_state.status("dummy"), ModuleStatus::Enabled);

    let s = build_with(VALID, &[], &["dummy"]).unwrap();
    assert_eq!(s.module_state.status("dummy"), ModuleStatus::Disabled);
    let s = build_with(VALID, &[], &[]).unwrap();
    assert_eq!(s.module_state.status("dummy"), ModuleStatus::Unavailable);
}

#[test]
fn an_unknown_module_name_stops_the_load() {
    let e = build_with(&format!("{VALID}\n[modules]\nvpl = true\n"), &[], &[]).unwrap_err();
    assert!(e.to_string().contains("modules.vpl: unknown module"), "{e}");
}

#[test]
fn enabling_a_module_this_binary_lacks_says_to_build_with_its_feature() {
    let e = build_with(&format!("{VALID}\n[modules]\ndummy = true\n"), &[], &[]).unwrap_err();
    assert!(e.to_string().contains("Cargo feature `dummy`"), "{e}");
}

#[test]
fn a_module_table_carries_its_settings_and_its_secret_comes_from_the_environment() {
    let toml = format!(
        "{VALID}\n[modules.dummy]\nenabled = true\nendpoint = \"https://api.example.com/v1\"\n"
    );
    let s = build_with(&toml, &[("VUMLO_DUMMY_API_KEY", "k-123")], &["dummy"]).unwrap();
    let dummy = s.module_state.get("dummy").unwrap();
    assert_eq!(dummy.settings["endpoint"], "https://api.example.com/v1");
    assert_eq!(dummy.secrets["api_key"].expose(), "k-123");

    let bad = toml.replace(
        "enabled = true",
        "enabled = true\napi_key = \"in-the-file\"",
    );
    let e = build_with(&bad, &[], &["dummy"]).unwrap_err().to_string();
    assert!(
        e.contains("VUMLO_DUMMY_API_KEY") && !e.contains("in-the-file"),
        "{e}"
    );
}

// --- the values the server has always checked --------------------------------------------

#[test]
fn valid_file_loads() {
    let s = build(VALID, &[]).unwrap();
    assert_eq!(s.database.max_connections, 5);
    assert_eq!(s.log.format, LogFormat::Json);
}

#[test]
fn environment_overrides_file() {
    let s = build(
        VALID,
        &[
            ("VUMLO__DATABASE__URL", "postgres://other/db"),
            ("VUMLO__LOG__FORMAT", "pretty"),
        ],
    )
    .unwrap();
    assert_eq!(s.database.url.expose(), "postgres://other/db");
    assert_eq!(s.log.format, LogFormat::Pretty);
}

#[test]
fn invalid_log_format_is_an_error() {
    let e = build(VALID, &[("VUMLO__LOG__FORMAT", "xml")]).unwrap_err();
    assert!(e.to_string().contains("xml"), "{e}");
}

#[test]
fn invalid_bind_addr_is_an_error() {
    assert!(build(VALID, &[("VUMLO__SERVER__BIND_ADDR", "nope")]).is_err());
}

#[test]
fn missing_key_is_an_error() {
    let e = build("[log]\nformat = \"pretty\"", &[]).unwrap_err();
    assert!(e.to_string().contains("server"), "{e}");
}

#[test]
fn auth_section_is_optional_and_defaults_are_safe() {
    let s = build(VALID, &[]).unwrap();
    assert_eq!(s.auth.password_min_length, 12);
    assert!(s.auth.cookie_secure);
    assert!(s.auth.allowed_domains.is_empty());
    assert_eq!(s.server.client_ip_header, None);
    assert_eq!(s.institution.name, "Example University");
}

#[test]
fn auth_values_come_from_the_environment() {
    let s = build(
        VALID,
        &[
            ("VUMLO__AUTH__PASSWORD_MIN_LENGTH", "16"),
            ("VUMLO__AUTH__COOKIE_SECURE", "false"),
            ("VUMLO__AUTH__ALLOWED_DOMAINS", "a.edu,b.edu"),
        ],
    )
    .unwrap();
    assert_eq!(s.auth.password_min_length, 16);
    assert!(!s.auth.cookie_secure);
    assert_eq!(s.auth.allowed_domains, ["a.edu", "b.edu"]);
}

#[test]
fn allowed_domains_must_be_lowercase_domain_names() {
    for bad in [
        "@example.edu",
        "Example.EDU",
        "example",
        "a b.edu",
        ".edu",
        "a..edu",
    ] {
        let e = build(VALID, &[("VUMLO__AUTH__ALLOWED_DOMAINS", bad)]).unwrap_err();
        assert!(e.to_string().contains("auth.allowed_domains"), "{bad}: {e}");
    }
}

#[test]
fn password_min_length_must_be_between_8_and_128() {
    for bad in ["7", "129"] {
        let e = build(VALID, &[("VUMLO__AUTH__PASSWORD_MIN_LENGTH", bad)]).unwrap_err();
        assert!(e.to_string().contains("password_min_length"), "{e}");
    }
    assert!(build(VALID, &[("VUMLO__AUTH__PASSWORD_MIN_LENGTH", "8")]).is_ok());
    assert!(build(VALID, &[("VUMLO__AUTH__PASSWORD_MIN_LENGTH", "128")]).is_ok());
}

#[test]
fn login_limits_have_defaults_come_from_the_environment_and_are_bounded() {
    let defaults = build(VALID, &[]).unwrap().auth;
    assert_eq!(
        (
            defaults.login_ip_burst,
            defaults.login_ip_replenish_seconds,
            defaults.login_account_max_attempts,
            defaults.login_account_window_minutes,
            defaults.max_concurrent_hashes
        ),
        (50, 1, 10, 15, 8)
    );
    let raised = build(
        VALID,
        &[
            ("VUMLO__AUTH__LOGIN_IP_BURST", "400"),
            ("VUMLO__AUTH__MAX_CONCURRENT_HASHES", "2"),
        ],
    )
    .unwrap()
    .auth;
    assert_eq!(
        (raised.login_ip_burst, raised.max_concurrent_hashes),
        (400, 2)
    );
    for (key, bad) in [
        ("LOGIN_IP_BURST", "0"),
        ("LOGIN_IP_REPLENISH_SECONDS", "0"),
        ("LOGIN_ACCOUNT_MAX_ATTEMPTS", "0"),
        ("LOGIN_ACCOUNT_WINDOW_MINUTES", "1441"),
        ("MAX_CONCURRENT_HASHES", "0"),
    ] {
        let e = build(VALID, &[(&format!("VUMLO__AUTH__{key}"), bad)]).unwrap_err();
        assert!(e.to_string().contains(&key.to_lowercase()), "{e}");
    }
}

#[test]
fn grading_scale_follows_the_mode_and_parses_both_values() {
    for (mode, scale) in [
        ("school", GradingScale::Twelve),
        ("university", GradingScale::Hundred),
    ] {
        let s = build(VALID, &[("VUMLO__INSTITUTION__MODE", mode)]).unwrap();
        assert_eq!(s.institution.grading_scale, scale, "{mode}");
    }
}

#[test]
fn an_explicit_grading_scale_wins_over_the_mode() {
    for (mode, text, scale) in [
        ("school", "hundred", GradingScale::Hundred),
        ("university", "twelve", GradingScale::Twelve),
    ] {
        let s = build(
            VALID,
            &[
                ("VUMLO__INSTITUTION__MODE", mode),
                ("VUMLO__INSTITUTION__GRADING_SCALE", text),
            ],
        )
        .unwrap();
        assert_eq!(s.institution.grading_scale, scale, "{mode} + {text}");
    }
}

#[test]
fn a_missing_mode_is_an_error_that_names_the_key_and_the_values() {
    let toml = VALID.replace("mode = \"university\"", "");
    let e = build(&toml, &[]).unwrap_err().to_string();
    for needle in ["institution.mode", "school", "university"] {
        assert!(e.contains(needle), "{needle}: {e}");
    }
}

#[test]
fn an_unknown_mode_is_an_error_that_names_the_key_and_the_value() {
    let e = build(VALID, &[("VUMLO__INSTITUTION__MODE", "college")])
        .unwrap_err()
        .to_string();
    for needle in ["institution.mode", "college", "school", "university"] {
        assert!(e.contains(needle), "{needle}: {e}");
    }
}

#[test]
fn both_modes_parse_and_the_environment_overrides_the_file() {
    assert_eq!(
        build(VALID, &[]).unwrap().institution.mode,
        InstitutionMode::University
    );
    let s = build(VALID, &[("VUMLO__INSTITUTION__MODE", "school")]).unwrap();
    assert_eq!(s.institution.mode, InstitutionMode::School);
}

#[test]
fn an_unknown_grading_scale_is_an_error_that_names_the_value() {
    let e = build(VALID, &[("VUMLO__INSTITUTION__GRADING_SCALE", "ten")]).unwrap_err();
    assert!(e.to_string().contains("ten"), "{e}");
}

#[test]
fn the_institution_profile_has_defaults() {
    let i = build(VALID, &[]).unwrap().institution;
    assert_eq!(i.short_name, "Example University");
    assert_eq!(i.language, Language::En);
    assert_eq!(i.time_zone, chrono_tz::UTC);
    assert_eq!(i.logo, None);
}

#[test]
fn the_institution_profile_is_read_from_the_file_and_the_environment() {
    let toml = VALID.replace(
        "mode = \"university\"",
        "mode = \"university\"\nshort_name = \" Example \"\nlanguage = \"uk\"\n\
         time_zone = \"Europe/Kyiv\"\nlogo = \"brand/logo.svg\"",
    );
    let i = build(&toml, &[]).unwrap().institution;
    assert_eq!(i.short_name, "Example");
    assert_eq!(i.language, Language::Uk);
    assert_eq!(i.time_zone, chrono_tz::Europe::Kyiv);
    assert_eq!(i.logo.as_deref(), Some("brand/logo.svg"));
    let i = build(&toml, &[("VUMLO__INSTITUTION__LANGUAGE", "en")])
        .unwrap()
        .institution;
    assert_eq!(i.language, Language::En);
}

#[test]
fn a_bad_institution_profile_value_stops_the_server_and_names_the_key_and_the_value() {
    for (key, bad) in [
        ("SHORT_NAME", ""),
        ("SHORT_NAME", &"x".repeat(51)),
        ("LANGUAGE", "fr"),
        ("TIME_ZONE", "Europe/Kyev"),
        ("TIME_ZONE", "+03:00"),
    ] {
        let e = build(VALID, &[(&format!("VUMLO__INSTITUTION__{key}"), bad)])
            .unwrap_err()
            .to_string();
        let needle = format!("institution.{}", key.to_lowercase());
        assert!(e.contains(&needle), "{key}={bad}: {e}");
    }
    let e = build(VALID, &[("VUMLO__INSTITUTION__TIME_ZONE", "Europe/Kyev")])
        .unwrap_err()
        .to_string();
    assert!(e.contains("Europe/Kyev"), "{e}");
}

const MAIL_FILE: &str = r#"
    [mail]
    from = "Vumlo <no-reply@lms.example.edu>"
    link_base = "https://lms.example.edu/"
    [mail.transport]
    kind = "file"
    dir = "/tmp/mail"
"#;

#[test]
fn google_is_optional_has_a_default_issuer_and_hides_its_secret() {
    assert!(build(VALID, &[]).unwrap().auth.google.is_none());
    let toml = format!(
        "{VALID}\n[auth.google]\nclient_id = \"id\"\nredirect_uri = \"https://lms.example.edu/auth/google/callback\"\n"
    );
    let google = build(
        &toml,
        &[("VUMLO__AUTH__GOOGLE__CLIENT_SECRET", "s3cret-value")],
    )
    .unwrap()
    .auth
    .google
    .unwrap();
    assert_eq!(google.issuer, "https://accounts.google.com");
    assert_eq!(google.client_secret.expose(), "s3cret-value");
    assert!(!format!("{google:?}").contains("s3cret-value"));
    // The secret is required: without it the section is refused.
    assert!(build(&toml, &[]).is_err());
}

#[test]
fn mail_is_optional_and_a_file_transport_loads() {
    assert!(build(VALID, &[]).unwrap().mail.is_none());
    let mail = build(&format!("{VALID}{MAIL_FILE}"), &[])
        .unwrap()
        .mail
        .unwrap();
    assert_eq!(mail.from, "Vumlo <no-reply@lms.example.edu>");
    assert_eq!(mail.link_base.as_deref(), Some("https://lms.example.edu"));
    assert!(matches!(mail.transport, MailTransport::File { ref dir } if dir.ends_with("mail")));
}

#[test]
fn a_bad_mail_section_stops_the_server_and_names_the_key() {
    let unknown = format!("{VALID}{}", MAIL_FILE.replace("dir =", "directory ="));
    assert!(build(&unknown, &[]).is_err());
    let bad_base = format!(
        "{VALID}{}",
        MAIL_FILE.replace("https://lms.example.edu/", "lms.example.edu/app")
    );
    let e = build(&bad_base, &[]).unwrap_err().to_string();
    assert!(e.contains("mail.link_base"), "{e}");
}

#[test]
fn public_origin_loses_the_trailing_slash_and_rejects_paths() {
    assert_eq!(
        build(VALID, &[]).unwrap().server.public_origin,
        "https://lms.example.edu"
    );
    for bad in ["lms.example.edu", "https://lms.example.edu/app", "ftp://x"] {
        let e = build(VALID, &[("VUMLO__SERVER__PUBLIC_ORIGIN", bad)]).unwrap_err();
        assert!(e.to_string().contains("public_origin"), "{bad}: {e}");
    }
}

#[test]
fn allowed_origins_default_to_none_and_come_from_a_list() {
    assert!(build(VALID, &[]).unwrap().server.allowed_origins.is_empty());
    let settings = build(
        VALID,
        &[(
            "VUMLO__SERVER__ALLOWED_ORIGINS",
            "https://app.example.edu/,http://localhost:5173,https://[::1]:8443",
        )],
    )
    .unwrap();
    assert_eq!(
        settings.server.allowed_origins,
        [
            "https://app.example.edu",
            "http://localhost:5173",
            "https://[::1]:8443"
        ]
    );
    let toml = VALID.replace(
        "public_origin = ",
        "allowed_origins = [\"https://a.test\"]\npublic_origin = ",
    );
    assert_eq!(
        build(&toml, &[]).unwrap().server.allowed_origins,
        ["https://a.test"]
    );
}

#[test]
fn invalid_origins_stop_the_server_and_name_the_key() {
    for bad in [
        "*",
        "https://*.example.edu",
        "null",
        "",
        "app.example.edu",
        "ftp://a.test",
        "https://a.test/x",
        "https://a.test?q=1",
        "https://a.test#f",
        "https://A.test",
        "https://a.test:443",
        "http://a.test:80",
        "https://a.test:0",
        "https://a.test:99999",
        "https://a.test:",
        "https://a.test:80a",
        "https://user@a.test",
        "https://a.test//",
        "https://",
        "https://[]",
    ] {
        let e = build(VALID, &[("VUMLO__SERVER__ALLOWED_ORIGINS", bad)]).unwrap_err();
        assert!(
            e.to_string().contains("server.allowed_origins"),
            "{bad:?}: {e}"
        );
        let e = build(VALID, &[("VUMLO__SERVER__PUBLIC_ORIGIN", bad)]).unwrap_err();
        assert!(
            e.to_string().contains("server.public_origin"),
            "{bad:?}: {e}"
        );
    }
    let e = build(
        VALID,
        &[("VUMLO__SERVER__ALLOWED_ORIGINS", "https://a.test,*")],
    )
    .unwrap_err();
    assert!(e.to_string().contains("server.allowed_origins"), "{e}");
}

fn with_editor(extra: &str) -> String {
    format!("{VALID}\n[editor]\n{extra}")
}

#[test]
fn the_editor_section_is_optional_and_video_is_on_by_default() {
    let s = build(VALID, &[]).unwrap();
    assert!(s.editor.profiles.is_empty());
    assert_eq!(s.editor.video_providers, ["youtube", "vimeo"]);
}

#[test]
fn editor_values_come_from_the_file_and_the_environment() {
    let toml = with_editor(
        "video_providers = [\"vimeo\"]\n[editor.profiles.option]\nfeatures = [\"links\"]",
    );
    let s = build(&toml, &[]).unwrap();
    assert_eq!(s.editor.video_providers, ["vimeo"]);
    assert_eq!(s.editor.profiles["option"].features, ["links"]);

    let s = build(
        VALID,
        &[
            ("VUMLO__EDITOR__VIDEO_PROVIDERS", "youtube"),
            (
                "VUMLO__EDITOR__PROFILES__STUDENT__FEATURES",
                "text_styles,links",
            ),
        ],
    )
    .unwrap();
    assert_eq!(s.editor.video_providers, ["youtube"]);
    assert_eq!(
        s.editor.profiles["student"].features,
        ["text_styles", "links"]
    );
}

#[test]
fn an_empty_provider_list_is_allowed_and_switches_video_off() {
    let s = build(&with_editor("video_providers = []"), &[]).unwrap();
    assert!(s.editor.video_providers.is_empty());
}

#[test]
fn an_unknown_key_in_an_editor_profile_is_a_parse_error_that_names_it() {
    let toml = with_editor("[editor.profiles.full]\nfeatures = []\ncolor = \"red\"");
    let e = build(&toml, &[]).unwrap_err();
    assert!(e.to_string().contains("color"), "{e}");
}

#[test]
fn settings_hold_the_editor_section_as_data_and_what_it_means_is_checked_later() {
    // `AppState::new` resolves the profiles and rejects these (tests/api.rs).
    for extra in [
        "[editor.profiles.wizard]\nfeatures = []",
        "[editor.profiles.full]\nfeatures = [\"telepathy\"]",
        "[editor.profiles.option]\nfeatures = [\"headings\"]",
        "video_providers = [\"dailymotion\"]",
    ] {
        build(&with_editor(extra), &[]).unwrap();
    }
    let s = build(VALID, &[("VUMLO__EDITOR__VIDEO_PROVIDERS", "vimeo,nope")]).unwrap();
    assert_eq!(s.editor.video_providers, ["vimeo", "nope"]);
}

#[test]
fn the_example_file_loads_as_it_is() {
    let text = include_str!("../../../../vumlo.example.toml");
    let s = build(text, &[]).unwrap();
    assert_eq!(s.config_version, SUPPORTED_CONFIG_VERSION);
    assert_eq!(s.institution.mode, InstitutionMode::University);
    assert!(
        s.module_state
            .statuses()
            .values()
            .all(|m| *m != ModuleStatus::Enabled)
    );
}

// --- the env file -------------------------------------------------------------------------

#[test]
fn an_env_file_has_pairs_comments_quotes_and_export() {
    let pairs = parse_env_file(
        "# comment\n\nDATABASE_URL=postgres://a/b\nexport X=\"two words\"\nY='q'\nbroken line\n",
    );
    assert_eq!(
        pairs,
        [
            ("DATABASE_URL".to_owned(), "postgres://a/b".to_owned()),
            ("X".to_owned(), "two words".to_owned()),
            ("Y".to_owned(), "q".to_owned()),
        ]
    );
}
