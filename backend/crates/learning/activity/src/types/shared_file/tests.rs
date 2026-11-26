use serde_json::json;

use super::*;

fn profiles() -> EffectiveProfiles {
    EffectiveProfiles::unrestricted()
}

fn doc(words: &str) -> Value {
    json!({
        "v": 1,
        "blocks": [{"id": "p", "type": "paragraph", "content": [{"type": "text", "text": words}]}],
        "attachments": []
    })
}

fn paths(errors: &DocumentErrors) -> Vec<String> {
    errors.iter().map(|error| error.path.clone()).collect()
}

#[test]
fn the_body_is_an_object_with_only_a_file_id_and_a_description() {
    let id = Uuid::now_v7();
    let stored = validate_body(
        &json!({"file_id": id.to_string().to_uppercase(), "description": doc("slides")}),
        &profiles(),
    )
    .unwrap();
    // The id is stored as a plain lowercase uuid and the description as a canonical document.
    assert_eq!(stored["file_id"], json!(id.to_string()));
    assert_eq!(stored["description"]["blocks"][0]["id"], "p");
    assert_eq!(validate_body(&json!({}), &profiles()), Ok(Map::new()));

    let not_an_object = validate_body(&json!([1]), &profiles()).unwrap_err();
    assert_eq!(paths(&not_an_object), [""]);
    let unknown = validate_body(&json!({"title": "x"}), &profiles()).unwrap_err();
    assert_eq!(paths(&unknown), ["title"]);
    let bad_id = validate_body(&json!({"file_id": "nope"}), &profiles()).unwrap_err();
    assert_eq!(paths(&bad_id), ["file_id"]);
    let bad_description =
        validate_body(&json!({"description": {"v": 2}}), &profiles()).unwrap_err();
    assert!(
        paths(&bad_description)
            .iter()
            .all(|path| path.starts_with("description")),
        "{:?}",
        paths(&bad_description)
    );
}

#[test]
fn the_files_of_a_shared_file_are_its_own_and_those_in_its_description() {
    let id = Uuid::now_v7();
    let body = json!({"file_id": id.to_string()});
    let refs = SharedFile
        .body_file_refs(body.as_object().unwrap())
        .unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].id, id);
    assert_eq!(refs[0].path, "body.file_id");
    // No usable id and no description: nothing is named.
    let none = json!({"file_id": "nope"});
    assert_eq!(
        SharedFile.body_file_refs(none.as_object().unwrap()),
        Ok(Vec::new())
    );
}

#[test]
fn a_shared_file_needs_a_file_before_it_is_published() {
    let with = json!({"file_id": Uuid::now_v7().to_string()});
    assert_eq!(
        SharedFile.check_publishable(with.as_object().unwrap()),
        Ok(())
    );
    for without in [
        json!({}),
        json!({"description": doc("x")}),
        json!({"file_id": "nope"}),
    ] {
        assert!(
            SharedFile
                .check_publishable(without.as_object().unwrap())
                .is_err(),
            "{without}"
        );
    }
    // A new one starts with no body at all.
    assert_eq!(SharedFile.default_body(), Map::new());
    assert_eq!(SharedFile.id(), "shared_file");
    assert!(SharedFile.graded().is_none());
    assert_eq!(SharedFile.completion(), CompletionPolicy::Marked);
}
