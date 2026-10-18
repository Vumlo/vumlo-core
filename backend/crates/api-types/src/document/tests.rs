use serde_json::{Value, json};
use utoipa::OpenApi;

use super::*;

fn run(text: &str) -> Inline {
    Inline::Text(TextRun {
        text: text.into(),
        styles: Styles::default(),
    })
}

fn sample() -> Value {
    json!({
        "v": 1,
        "blocks": [
            {"id": "a", "type": "paragraph", "content": [
                {"type": "text", "text": "Hi ", "styles": {"bold": true}},
                {"type": "link", "href": "https://x.edu", "content": [{"type": "text", "text": "x"}]},
                {"type": "math", "source": "x^2"}
            ]},
            {"id": "b", "type": "heading", "props": {"level": 2}},
            {"id": "c", "type": "bulleted_list_item", "children": [
                {"id": "d", "type": "check_list_item", "props": {"checked": true}}
            ]},
            {"id": "e", "type": "numbered_list_item"},
            {"id": "f", "type": "quote"},
            {"id": "g", "type": "divider"},
            {"id": "h", "type": "table", "props": {"header_row": true, "rows": [
                [[{"type": "text", "text": "a"}], []]
            ]}},
            {"id": "i", "type": "code", "props": {"language": "rust", "source": "fn main() {}"}},
            {"id": "j", "type": "math", "props": {"source": "E=mc^2"}},
            {"id": "k", "type": "mermaid", "props": {"source": "graph TD; A-->B"}},
            {"id": "l", "type": "video", "props": {"provider": "youtube", "video_id": "dQw4w9WgXcQ"}}
        ],
        "attachments": []
    })
}

#[test]
fn a_document_with_every_block_and_inline_kind_round_trips() {
    let value = sample();
    let document: Document = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(document.blocks.len(), 11);
    assert_eq!(serde_json::to_value(&document).unwrap(), value);
}

#[test]
fn empty_props_content_and_children_are_left_out() {
    let paragraph = Block::Paragraph {
        id: "a".into(),
        content: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(&paragraph).unwrap(),
        json!({"id": "a", "type": "paragraph"})
    );
    let list = Block::BulletedListItem {
        id: "b".into(),
        content: vec![run("x")],
        children: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(&list).unwrap(),
        json!({"id": "b", "type": "bulleted_list_item", "content": [{"type": "text", "text": "x"}]})
    );
    assert_eq!(
        serde_json::to_value(Document::empty()).unwrap(),
        json!({"v": 1, "blocks": [], "attachments": []})
    );
}

#[test]
fn omitted_attachments_read_as_empty() {
    let document: Document = serde_json::from_value(json!({"v": 1, "blocks": []})).unwrap();
    assert_eq!(document, Document::empty());
}

#[test]
fn an_unknown_block_or_inline_type_does_not_deserialize() {
    for bad in [
        json!({"v": 1, "blocks": [{"id": "a", "type": "image"}]}),
        json!({"v": 1, "blocks": [{"id": "a", "type": "html"}]}),
        json!({"v": 1, "blocks": [{"id": "a", "type": "paragraph",
                "content": [{"type": "emoji", "text": "x"}]}]}),
    ] {
        assert!(serde_json::from_value::<Document>(bad).is_err());
    }
}

#[test]
fn accessors_read_content_and_children() {
    let document: Document = serde_json::from_value(sample()).unwrap();
    assert_eq!(document.blocks[0].id(), "a");
    assert_eq!(document.blocks[0].content().len(), 3);
    assert_eq!(document.blocks[2].children().len(), 1);
    assert!(document.blocks[6].content().is_empty());
    assert!(document.blocks[6].children().is_empty());
}

#[test]
fn as_str_and_from_name_agree_with_serde() {
    fn check<T>(all: &[T], name: fn(T) -> &'static str, from: fn(&str) -> Option<T>)
    where
        T: Copy + PartialEq + std::fmt::Debug + Serialize,
    {
        for &item in all {
            assert_eq!(serde_json::to_value(item).unwrap(), name(item));
            assert_eq!(from(name(item)), Some(item));
        }
        assert_eq!(from("nope"), None);
    }
    check(
        CodeLanguage::ALL,
        CodeLanguage::as_str,
        CodeLanguage::from_name,
    );
    check(
        VideoProvider::ALL,
        VideoProvider::as_str,
        VideoProvider::from_name,
    );
    check(Feature::ALL, Feature::as_str, Feature::from_name);
    check(
        ProfileName::ALL,
        ProfileName::as_str,
        ProfileName::from_name,
    );
    assert_eq!(CodeLanguage::ALL.len(), 21);
    assert_eq!(Feature::ALL.len(), 11);
}

#[test]
fn the_names_on_the_wire_are_the_documented_ones() {
    assert_eq!(
        serde_json::to_value(CodeLanguage::Csharp).unwrap(),
        "csharp"
    );
    assert_eq!(serde_json::to_value(CodeLanguage::Cpp).unwrap(), "cpp");
    assert_eq!(serde_json::to_value(VideoProvider::Vimeo).unwrap(), "vimeo");
    assert_eq!(
        serde_json::to_value(Feature::TextStyles).unwrap(),
        "text_styles"
    );
    assert_eq!(serde_json::to_value(ProfileName::Option).unwrap(), "option");
}

#[test]
fn the_openapi_document_has_the_typed_schemas() {
    #[derive(OpenApi)]
    #[openapi(components(schemas(Document, EditorProfiles)))]
    struct Doc;

    let doc = serde_json::to_value(Doc::openapi()).unwrap();
    let schemas = &doc["components"]["schemas"];
    for name in [
        "Document",
        "Block",
        "Inline",
        "TextRun",
        "Styles",
        "VideoProps",
        "CodeLanguage",
        "VideoProvider",
        "Feature",
        "ProfileName",
        "EditorProfile",
        "DocumentLimits",
    ] {
        assert!(schemas[name].is_object(), "{name} missing");
    }
}
