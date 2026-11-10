use super::*;

#[test]
fn the_client_config_types_describe_every_field() {
    let ts = client_config_types().unwrap();
    for needle in [
        "export type ClientConfig = {",
        "api_url: string;",
        "public_origin: string;",
        "storage: \"local\" | \"s3\";",
        "max_size: number;",
        "language: \"en\" | \"uk\";",
        "grading_scale: \"twelve\" | \"hundred\";",
        "accent: \"violet\" | \"blue\" | \"teal\" | \"green\" | \"orange\" | \"red\";",
        "modules: { [name: string]: \"enabled\" | \"disabled\" | \"unavailable\" };",
        "web_modules: (string)[];",
    ] {
        assert!(ts.contains(needle), "{needle}\n{ts}");
    }
}

#[test]
fn a_shape_the_emitter_does_not_know_is_an_error() {
    let schema = serde_json::json!({"type": "object", "properties": {"x": {"not": {}}}});
    assert!(ts_type(&schema, 0).is_err());
}
