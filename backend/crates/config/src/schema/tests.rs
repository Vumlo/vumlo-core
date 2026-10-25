use super::*;

#[test]
fn every_key_is_marked_public_private_or_secret() {
    assert_eq!(unmarked(), Vec::<&str>::new());
}

#[test]
fn the_public_keys_are_the_institution_profile_and_the_origin() {
    let mut public = public_paths();
    public.sort_unstable();
    assert_eq!(
        public,
        [
            "institution.grading_scale",
            "institution.language",
            "institution.name",
            "institution.short_name",
            "institution.time_zone",
            "server.public_origin",
        ]
    );
}

#[test]
fn the_secret_keys_are_the_database_url_google_the_smtp_password_and_the_plugin_key() {
    let secrets: Vec<_> = secret_paths().into_iter().collect();
    assert_eq!(
        secrets,
        [
            "auth.google.client_secret",
            "database.url",
            "mail.transport.password",
            "plugins.secret_key",
        ]
    );
}

#[test]
fn a_secret_is_read_from_the_environment_under_a_predictable_name() {
    assert_eq!(env_name("database.url"), "DATABASE_URL");
    assert_eq!(
        env_name("auth.google.client_secret"),
        "VUMLO__AUTH__GOOGLE__CLIENT_SECRET"
    );
}

#[test]
fn unknown_keys_are_found_at_any_depth_and_map_entries_are_known() {
    let value = serde_json::json!({
        "server": {"public_origin": "https://x.test", "nope": 1},
        "institution": {"accent": "teal"},
        "editor": {"profiles": {"student": {"features": []}}},
        "modules": {"anything": {"goes": true}},
        "mail": {"transport": {"kind": "smtp", "host": "h", "password": "p"}},
    });
    assert_eq!(unknown_keys(&value), ["institution.accent", "server.nope"]);
}

#[test]
fn secret_looking_names_are_recognised_by_their_last_segment() {
    assert!(looks_secret("mail.transport.api_key"));
    assert!(looks_secret("modules.x.Token"));
    assert!(!looks_secret("auth.allowed_domains"));
}
