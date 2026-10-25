use serde_json::json;

use super::*;

fn doc(blocks: serde_json::Value) -> Document {
    serde_json::from_value(json!({"v": 1, "blocks": blocks})).unwrap()
}

#[test]
fn blocks_are_lines_and_children_follow_their_block() {
    let document = doc(json!([
        {"id": "a", "type": "heading", "props": {"level": 1},
            "content": [{"type": "text", "text": "Title"}]},
        {"id": "b", "type": "bulleted_list_item", "content": [{"type": "text", "text": "one"}],
            "children": [{"id": "c", "type": "numbered_list_item",
                "content": [{"type": "text", "text": "nested"}]}]},
        {"id": "d", "type": "paragraph", "content": [{"type": "text", "text": "last"}]},
    ]));
    assert_eq!(plain_text(&document), "Title\none\nnested\nlast");
}

#[test]
fn inline_text_link_text_and_formulas_are_kept_as_they_are() {
    let document = doc(json!([
        {"id": "a", "type": "paragraph", "content": [
            {"type": "text", "text": "See ", "styles": {"bold": true}},
            {"type": "link", "href": "https://x.edu",
                "content": [{"type": "text", "text": "here"}, {"type": "text", "text": "!"}]},
            {"type": "text", "text": " and "},
            {"type": "math", "source": "x^2"},
        ]},
    ]));
    assert_eq!(plain_text(&document), "See here! and x^2");
}

#[test]
fn formulas_diagrams_and_code_give_their_source() {
    let document = doc(json!([
        {"id": "a", "type": "math", "props": {"source": "E=mc^2"}},
        {"id": "b", "type": "mermaid", "props": {"source": "graph TD; A-->B"}},
        {"id": "c", "type": "code", "props": {"language": "rust", "source": "fn main() {}"}},
    ]));
    assert_eq!(
        plain_text(&document),
        "E=mc^2\ngraph TD; A-->B\nfn main() {}"
    );
}

#[test]
fn a_table_has_tabs_between_cells_and_newlines_between_rows() {
    let document = doc(json!([
        {"id": "a", "type": "paragraph", "content": [{"type": "text", "text": "before"}]},
        {"id": "b", "type": "table", "props": {"header_row": true, "rows": [
            [[{"type": "text", "text": "a"}], [{"type": "text", "text": "b"}]],
            [[{"type": "math", "source": "c"}], []],
        ]}},
    ]));
    assert_eq!(plain_text(&document), "before\na\tb\nc\t");
}

#[test]
fn a_divider_and_a_video_give_nothing_not_even_a_line() {
    let document = doc(json!([
        {"id": "a", "type": "paragraph", "content": [{"type": "text", "text": "x"}]},
        {"id": "b", "type": "divider"},
        {"id": "c", "type": "video", "props": {"provider": "vimeo", "video_id": "76979871"}},
        {"id": "d", "type": "paragraph", "content": [{"type": "text", "text": "y"}]},
    ]));
    assert_eq!(plain_text(&document), "x\ny");
}

#[test]
fn an_empty_document_is_empty_and_an_empty_paragraph_is_an_empty_line() {
    assert_eq!(plain_text(&Document::empty()), "");
    let document = doc(json!([
        {"id": "a", "type": "paragraph", "content": [{"type": "text", "text": "x"}]},
        {"id": "b", "type": "paragraph"},
        {"id": "c", "type": "paragraph", "content": [{"type": "text", "text": "y"}]},
    ]));
    assert_eq!(plain_text(&document), "x\n\ny");
}
