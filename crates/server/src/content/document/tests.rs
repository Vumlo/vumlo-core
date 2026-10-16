use api_types::{Feature, ProfileName, VideoProvider};
use serde_json::{Value, json};

use super::*;
use crate::settings::{EditorSettings, ProfileSettings};

fn doc(blocks: Value) -> Value {
    json!({"v": 1, "blocks": blocks, "attachments": []})
}

fn paragraph(id: &str, text: &str) -> Value {
    json!({"id": id, "type": "paragraph", "content": [{"type": "text", "text": text}]})
}

fn full() -> EffectiveProfiles {
    EffectiveProfiles::unrestricted()
}

fn validate(value: &Value) -> Result<ValidDocument, DocumentErrors> {
    validate_document(value, ProfileName::Full, &full())
}

/// The error paths in the order they were found.
fn paths(value: &Value) -> Vec<String> {
    validate(value)
        .expect_err("the document should be refused")
        .iter()
        .map(|error| error.path.clone())
        .collect()
}

fn single(value: &Value) -> (String, &'static str) {
    let errors = validate(value).expect_err("the document should be refused");
    let errors: Vec<_> = errors.iter().collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    (errors[0].path.clone(), errors[0].message)
}

fn sample() -> Value {
    doc(json!([
        {"id": "a", "type": "paragraph", "content": [
            {"type": "text", "text": "Hi ", "styles": {"bold": true, "code": true}},
            {"type": "link", "href": "https://x.edu/a?b=c", "content": [{"type": "text", "text": "x"}]},
            {"type": "math", "source": "x^2"}
        ]},
        {"id": "b", "type": "heading", "props": {"level": 3}, "content": [{"type": "text", "text": "T"}]},
        {"id": "c", "type": "bulleted_list_item", "children": [
            {"id": "d", "type": "check_list_item", "props": {"checked": true}}
        ]},
        {"id": "e", "type": "numbered_list_item"},
        {"id": "f", "type": "quote", "content": [{"type": "text", "text": "q"}]},
        {"id": "g", "type": "divider"},
        {"id": "h", "type": "table", "props": {"header_row": true, "rows": [
            [[{"type": "text", "text": "a"}], [{"type": "math", "source": "b"}]],
            [[], []]
        ]}},
        {"id": "i", "type": "code", "props": {"language": "rust", "source": "fn main() {}"}},
        {"id": "j", "type": "math", "props": {"source": "E=mc^2"}},
        {"id": "k", "type": "mermaid", "props": {"source": "graph TD; A-->B"}},
        {"id": "l", "type": "video", "props": {"provider": "youtube", "video_id": "dQw4w9WgXcQ"}},
        {"id": "m", "type": "video", "props": {"provider": "vimeo", "video_id": "76979871"}}
    ]))
}

#[test]
fn every_block_and_inline_kind_is_valid_and_stored_as_sent() {
    let value = sample();
    let valid = validate(&value).unwrap();
    assert_eq!(valid.as_document().blocks.len(), 12);
    assert_eq!(Value::Object(valid.as_body().clone()), value);
}

#[test]
fn the_empty_document_is_valid_and_is_what_an_item_starts_with() {
    let empty = ValidDocument::empty();
    assert_eq!(
        Value::Object(empty.as_body().clone()),
        json!({"v": 1, "blocks": [], "attachments": []})
    );
    assert_eq!(validate(&doc(json!([]))).unwrap(), empty);
}

#[test]
fn keys_that_are_empty_may_be_sent_and_are_stored_without_them() {
    let sent = json!({"v": 1, "blocks": [
        {"id": "a", "type": "paragraph", "props": {}, "content": [], "children": []},
        {"id": "b", "type": "divider", "props": {}, "content": [], "children": []},
    ]});
    let valid = validate(&sent).unwrap();
    assert_eq!(
        Value::Object(valid.as_body().clone()),
        json!({"v": 1, "blocks": [
            {"id": "a", "type": "paragraph"},
            {"id": "b", "type": "divider"},
        ], "attachments": []})
    );
}

#[test]
fn an_unknown_code_language_is_stored_as_text_and_a_known_one_is_kept() {
    let block = |language: &str| {
        doc(json!([{"id": "c", "type": "code", "props": {"language": language, "source": "x"}}]))
    };
    for (sent, stored) in [
        ("klingon", "text"),
        ("Python", "text"),
        ("python", "python"),
        ("cpp", "cpp"),
    ] {
        let valid = validate(&block(sent)).unwrap();
        assert_eq!(
            valid.as_body()["blocks"][0]["props"]["language"],
            stored,
            "{sent}"
        );
    }
}

#[test]
fn each_rule_refuses_with_the_path_of_the_problem() {
    let level =
        |level: Value| doc(json!([{"id": "h", "type": "heading", "props": {"level": level}}]));
    let table = |rows: Value| {
        doc(json!([{"id": "t", "type": "table", "props": {"header_row": false, "rows": rows}}]))
    };
    let link = |href: &str| {
        doc(json!([{"id": "p", "type": "paragraph",
            "content": [{"type": "link", "href": href, "content": []}]}]))
    };
    let video = |provider: &str, id: &str| {
        doc(json!([{"id": "v", "type": "video", "props": {"provider": provider, "video_id": id}}]))
    };
    let source = |kind: &str, bytes: usize| {
        let props = if kind == "code" {
            json!({"language": "text", "source": "x".repeat(bytes)})
        } else {
            json!({"source": "x".repeat(bytes)})
        };
        doc(json!([{"id": "s", "type": kind, "props": props}]))
    };
    let deep = |levels: usize| {
        let mut block = json!({"id": format!("n{levels}"), "type": "bulleted_list_item"});
        for level in (1..levels).rev() {
            block = json!({"id": format!("n{level}"), "type": "bulleted_list_item", "children": [block]});
        }
        doc(json!([block]))
    };
    let wide = |rows: usize, columns: usize| table(json!(vec![vec![json!([]); columns]; rows]));

    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "duplicate id",
            doc(json!([paragraph("a", "x"), paragraph("a", "y")])),
            "blocks[1].id",
        ),
        (
            "duplicate id in a child",
            doc(json!([
            {"id": "a", "type": "bulleted_list_item", "children": [paragraph("a", "x")]}])),
            "blocks[0].children[0].id",
        ),
        (
            "id with a space",
            doc(json!([paragraph("a b", "x")])),
            "blocks[0].id",
        ),
        ("empty id", doc(json!([paragraph("", "x")])), "blocks[0].id"),
        (
            "id of 65 characters",
            doc(json!([paragraph(&"a".repeat(65), "x")])),
            "blocks[0].id",
        ),
        (
            "id that is not a string",
            doc(json!([{"id": 3, "type": "divider"}])),
            "blocks[0].id",
        ),
        ("level 0", level(json!(0)), "blocks[0].props.level"),
        ("level 4", level(json!(4)), "blocks[0].props.level"),
        ("level as text", level(json!("2")), "blocks[0].props.level"),
        (
            "rows of different length",
            table(json!([[[], []], [[]]])),
            "blocks[0].props.rows[1]",
        ),
        ("101 rows", wide(101, 1), "blocks[0].props.rows"),
        ("21 columns", wide(1, 21), "blocks[0].props.rows[0]"),
        (
            "rows that are not an array",
            table(json!("x")),
            "blocks[0].props.rows",
        ),
        (
            "depth 7",
            deep(7),
            "blocks[0].children[0].children[0].children[0].children[0].children[0].children[0]",
        ),
        (
            "href javascript",
            link("javascript:alert(1)"),
            "blocks[0].content[0].href",
        ),
        (
            "href data",
            link("data:text/html,x"),
            "blocks[0].content[0].href",
        ),
        (
            "href ftp",
            link("ftp://x.edu/f"),
            "blocks[0].content[0].href",
        ),
        (
            "href without a scheme",
            link("x.edu"),
            "blocks[0].content[0].href",
        ),
        (
            "href with nothing after the scheme",
            link("https://"),
            "blocks[0].content[0].href",
        ),
        (
            "href with a space",
            link("https://x.edu/a b"),
            "blocks[0].content[0].href",
        ),
        (
            "href of 2049 characters",
            link(&format!("https://{}", "a".repeat(2041))),
            "blocks[0].content[0].href",
        ),
        (
            "youtube id too short",
            video("youtube", "abc"),
            "blocks[0].props.video_id",
        ),
        (
            "youtube id with a bad character",
            video("youtube", "dQw4w9WgXc!"),
            "blocks[0].props.video_id",
        ),
        (
            "vimeo id with letters",
            video("vimeo", "abc"),
            "blocks[0].props.video_id",
        ),
        (
            "vimeo id of 13 digits",
            video("vimeo", "1234567890123"),
            "blocks[0].props.video_id",
        ),
        (
            "vimeo id that is empty",
            video("vimeo", ""),
            "blocks[0].props.video_id",
        ),
        (
            "unknown provider",
            video("dailymotion", "x"),
            "blocks[0].props.provider",
        ),
        (
            "unknown style key",
            doc(json!([{"id": "p", "type": "paragraph",
            "content": [{"type": "text", "text": "x", "styles": {"underline": true}}]}])),
            "blocks[0].content[0].styles",
        ),
        (
            "style that is not a boolean",
            doc(json!([{"id": "p", "type": "paragraph",
            "content": [{"type": "text", "text": "x", "styles": {"bold": "yes"}}]}])),
            "blocks[0].content[0].styles.bold",
        ),
        (
            "unknown block type",
            doc(json!([{"id": "a", "type": "carousel"}])),
            "blocks[0].type",
        ),
        (
            "unknown block property",
            doc(json!([{"id": "a", "type": "paragraph", "color": "red"}])),
            "blocks[0]",
        ),
        (
            "unknown prop",
            doc(json!([{"id": "a", "type": "paragraph", "props": {"color": "red"}}])),
            "blocks[0].props",
        ),
        (
            "unknown inline type",
            doc(json!([{"id": "a", "type": "paragraph",
            "content": [{"type": "emoji", "text": "x"}]}])),
            "blocks[0].content[0].type",
        ),
        (
            "unknown document key",
            json!({"v": 1, "blocks": [], "title": "x"}),
            "",
        ),
        (
            "content on a divider",
            doc(json!([{"id": "a", "type": "divider",
            "content": [{"type": "text", "text": "x"}]}])),
            "blocks[0].content",
        ),
        (
            "children on a paragraph",
            doc(json!([{"id": "a", "type": "paragraph",
            "children": [paragraph("b", "x")]}])),
            "blocks[0].children",
        ),
        (
            "attachments",
            json!({"v": 1, "blocks": [], "attachments": ["8c1b6a5e-0000-7000-8000-000000000000"]}),
            "attachments",
        ),
        (
            "attachments that are not an array",
            json!({"v": 1, "blocks": [], "attachments": 3}),
            "attachments",
        ),
        ("version 2", json!({"v": 2, "blocks": []}), "v"),
        ("no version", json!({"blocks": []}), "v"),
        (
            "blocks that are not an array",
            json!({"v": 1, "blocks": {}}),
            "blocks",
        ),
        (
            "code of 64 KiB and a byte",
            source("code", MAX_CODE_BYTES + 1),
            "blocks[0].props.source",
        ),
        (
            "math of 16 KiB and a byte",
            source("math", MAX_FORMULA_BYTES + 1),
            "blocks[0].props.source",
        ),
        (
            "mermaid of 16 KiB and a byte",
            source("mermaid", MAX_FORMULA_BYTES + 1),
            "blocks[0].props.source",
        ),
        (
            "inline math of 16 KiB and a byte",
            doc(json!([{"id": "a", "type": "paragraph",
            "content": [{"type": "math", "source": "x".repeat(MAX_FORMULA_BYTES + 1)}]}])),
            "blocks[0].content[0].source",
        ),
    ];
    for (name, value, expected) in cases {
        assert_eq!(paths(&value), [expected], "{name}");
    }

    // The limits themselves are fine.
    for value in [
        source("code", MAX_CODE_BYTES),
        source("math", MAX_FORMULA_BYTES),
        source("mermaid", MAX_FORMULA_BYTES),
        deep(6),
        wide(100, 20),
        link(&format!("https://{}", "a".repeat(2040))),
        level(json!(1)),
        level(json!(3)),
        video("vimeo", "123456789012"),
        link("HTTPS://X.EDU"),
    ] {
        validate(&value).unwrap();
    }
}

#[test]
fn images_and_html_are_not_available_yet() {
    for kind in ["image", "html"] {
        let value = doc(json!([{"id": "a", "type": kind}]));
        assert_eq!(
            single(&value),
            ("blocks[0].type".to_owned(), "is not available yet"),
            "{kind}"
        );
    }
}

#[test]
fn a_document_over_one_mebibyte_is_one_error_on_the_document() {
    let value = doc(json!([paragraph("a", &"x".repeat(MAX_DOCUMENT_BYTES))]));
    assert_eq!(single(&value), (String::new(), "is larger than 1 MiB"));
    let fields = validate(&value).unwrap_err().into_field_errors("body");
    assert_eq!(fields[0].field, "body");
}

#[test]
fn at_most_twenty_problems_are_reported() {
    let blocks: Vec<Value> = (0..30)
        .map(|i| json!({"id": format!("h{i}"), "type": "heading", "props": {"level": 9}}))
        .collect();
    let found = paths(&doc(json!(blocks)));
    assert_eq!(found.len(), MAX_ERRORS);
    assert_eq!(found[0], "blocks[0].props.level");
}

#[test]
fn field_errors_are_prefixed_and_the_document_itself_is_the_prefix() {
    let errors = validate(&doc(json!([paragraph("a", "x"), paragraph("a", "y")])))
        .unwrap_err()
        .into_field_errors("body");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].field, "body.blocks[1].id");
    assert_eq!(errors[0].message, "must be unique in the document");
}

#[test]
fn nothing_that_was_sent_comes_back_in_the_errors() {
    let value = doc(json!([
        {"id": "SECRET id", "type": "paragraph", "SECRET_KEY": 1, "props": {"SECRET_PROP": 1},
            "content": [
                {"type": "text", "text": "SECRET_TEXT", "styles": {"SECRET_STYLE": true}},
                {"type": "link", "href": "javascript:SECRET_HREF", "content": [{"type": "SECRET_RUN"}]},
                {"type": "SECRET_INLINE", "text": "SECRET_TEXT"},
            ]},
        {"id": "b", "type": "SECRET_TYPE"},
        {"id": "c", "type": "video", "props": {"provider": "SECRET_PROVIDER", "video_id": "SECRET_VIDEO"}},
        {"id": "d", "type": "video", "props": {"provider": "youtube", "video_id": "SECRET_VIDEO"}},
        {"id": "e", "type": "code", "props": {"language": "SECRET_LANG", "source": 5, "SECRET_PROP": 1}},
        {"id": "e", "type": "table", "props": {"header_row": "SECRET_BOOL", "rows": [[[], []], [["SECRET_CELL"]]]}},
    ]));
    let errors = validate(&value).unwrap_err();
    assert!(errors.iter().count() > 5);
    let shown = format!("{errors:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
}

// --- profiles ----------------------------------------------------------------------------------

/// One document per kind of block and inline run, each holding a single block.
fn kinds() -> Vec<(&'static str, Value)> {
    let inline = |run: Value| doc(json!([{"id": "p", "type": "paragraph", "content": [run]}]));
    vec![
        ("paragraph", doc(json!([paragraph("p", "x")]))),
        (
            "styled text",
            inline(json!({"type": "text", "text": "x", "styles": {"italic": true}})),
        ),
        (
            "link",
            inline(json!({"type": "link", "href": "https://x.edu", "content": []})),
        ),
        (
            "styled link text",
            inline(json!({"type": "link", "href": "https://x.edu",
            "content": [{"type": "text", "text": "x", "styles": {"bold": true}}]})),
        ),
        (
            "inline math",
            inline(json!({"type": "math", "source": "x"})),
        ),
        ("quote", doc(json!([{"id": "q", "type": "quote"}]))),
        (
            "heading",
            doc(json!([{"id": "h", "type": "heading", "props": {"level": 1}}])),
        ),
        (
            "bulleted list item",
            doc(json!([{"id": "l", "type": "bulleted_list_item"}])),
        ),
        (
            "numbered list item",
            doc(json!([{"id": "l", "type": "numbered_list_item"}])),
        ),
        (
            "check list item",
            doc(json!([{"id": "l", "type": "check_list_item", "props": {"checked": false}}])),
        ),
        ("divider", doc(json!([{"id": "d", "type": "divider"}]))),
        (
            "table",
            doc(
                json!([{"id": "t", "type": "table", "props": {"header_row": false, "rows": [[[]]]}}]),
            ),
        ),
        (
            "table cell with a link",
            doc(
                json!([{"id": "t", "type": "table", "props": {"header_row": false,
            "rows": [[[{"type": "link", "href": "https://x.edu", "content": []}]]]}}]),
            ),
        ),
        (
            "code",
            doc(json!([{"id": "c", "type": "code", "props": {"language": "rust", "source": "x"}}])),
        ),
        (
            "math",
            doc(json!([{"id": "m", "type": "math", "props": {"source": "x"}}])),
        ),
        (
            "mermaid",
            doc(json!([{"id": "m", "type": "mermaid", "props": {"source": "x"}}])),
        ),
        (
            "video",
            doc(
                json!([{"id": "v", "type": "video", "props": {"provider": "youtube", "video_id": "dQw4w9WgXcQ"}}]),
            ),
        ),
    ]
}

/// The table of ADR-0025 and ADR-0027, written out once more so a change to the code is noticed.
fn allowed(profile: ProfileName, kind: &str) -> bool {
    match profile {
        ProfileName::Full | ProfileName::Question => true,
        ProfileName::Student => kind != "video",
        ProfileName::Option => matches!(
            kind,
            "paragraph"
                | "styled text"
                | "link"
                | "styled link text"
                | "inline math"
                | "code"
                | "math"
        ),
    }
}

#[test]
fn every_profile_allows_exactly_the_documented_kinds() {
    let profiles = full();
    for profile in ProfileName::ALL {
        for (kind, value) in kinds() {
            let result = validate_document(&value, *profile, &profiles);
            assert_eq!(
                result.is_ok(),
                allowed(*profile, kind),
                "{} / {kind}: {result:?}",
                profile.as_str()
            );
        }
    }
}

#[test]
fn a_refused_kind_is_reported_where_it_stands() {
    let profiles = full();
    let heading = doc(json!([{"id": "h", "type": "heading", "props": {"level": 1}}]));
    let errors = validate_document(&heading, ProfileName::Option, &profiles).unwrap_err();
    let errors: Vec<_> = errors
        .iter()
        .map(|e| (e.path.as_str(), e.message))
        .collect();
    assert_eq!(errors, [("blocks[0]", "is not allowed here")]);

    let bold = doc(json!([{"id": "p", "type": "paragraph", "content": [
        {"type": "text", "text": "a"},
        {"type": "link", "href": "https://x.edu", "content": [{"type": "text", "text": "b", "styles": {"bold": true}}]},
    ]}]));
    assert!(validate_document(&bold, ProfileName::Option, &profiles).is_ok());
    let narrowed = narrowed(ProfileName::Option, &["links"]);
    let errors = validate_document(&bold, ProfileName::Option, &narrowed).unwrap_err();
    let found: Vec<_> = errors.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(found, ["blocks[0].content[1].content[0].styles"]);
}

#[test]
fn the_option_profile_is_exactly_one_block_without_children() {
    let profiles = full();
    let check = |value: &Value| validate_document(value, ProfileName::Option, &profiles);
    assert!(check(&doc(json!([paragraph("a", "x")]))).is_ok());
    for blocks in [json!([]), json!([paragraph("a", "x"), paragraph("b", "y")])] {
        let errors = check(&doc(blocks)).unwrap_err();
        let found: Vec<_> = errors.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(found, ["blocks"]);
    }
    // A list item is refused as a feature, and its children are refused on their own.
    let nested =
        doc(json!([{"id": "l", "type": "bulleted_list_item", "children": [paragraph("c", "x")]}]));
    let errors = check(&nested).unwrap_err();
    let found: Vec<_> = errors.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(found, ["blocks[0]", "blocks[0].children"]);
    // The other profiles take any number of blocks.
    for profile in [
        ProfileName::Full,
        ProfileName::Question,
        ProfileName::Student,
    ] {
        assert!(validate_document(&doc(json!([])), profile, &profiles).is_ok());
    }
}

fn narrowed(profile: ProfileName, features: &[&str]) -> EffectiveProfiles {
    let mut settings = EditorSettings::default();
    settings.profiles.insert(
        profile.as_str().to_owned(),
        ProfileSettings {
            features: features.iter().map(|f| (*f).to_owned()).collect(),
        },
    );
    EffectiveProfiles::resolve(&settings).unwrap()
}

#[test]
fn narrowing_by_the_configuration_refuses_what_the_code_profile_allows() {
    let table = kinds()
        .into_iter()
        .find(|(kind, _)| *kind == "table")
        .unwrap()
        .1;
    assert!(validate(&table).is_ok());
    let profiles = narrowed(ProfileName::Full, &["headings", "lists"]);
    let errors = validate_document(&table, ProfileName::Full, &profiles).unwrap_err();
    assert_eq!(errors.iter().next().unwrap().path, "blocks[0]");
    // Only the narrowed profile changed.
    assert!(validate_document(&table, ProfileName::Question, &profiles).is_ok());
    assert_eq!(
        profiles
            .profile(ProfileName::Full)
            .features
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [Feature::Headings, Feature::Lists]
    );
}

#[test]
fn video_providers_can_be_narrowed_and_an_empty_list_switches_video_off() {
    let youtube = kinds()
        .into_iter()
        .find(|(kind, _)| *kind == "video")
        .unwrap()
        .1;
    let vimeo = doc(
        json!([{"id": "v", "type": "video", "props": {"provider": "vimeo", "video_id": "76979871"}}]),
    );

    let mut settings = EditorSettings {
        video_providers: vec!["vimeo".to_owned()],
        ..EditorSettings::default()
    };
    let only_vimeo = EffectiveProfiles::resolve(&settings).unwrap();
    let errors = validate_document(&youtube, ProfileName::Full, &only_vimeo).unwrap_err();
    assert_eq!(
        errors.iter().next().unwrap().path,
        "blocks[0].props.provider"
    );
    assert!(validate_document(&vimeo, ProfileName::Full, &only_vimeo).is_ok());

    settings.video_providers = Vec::new();
    let off = EffectiveProfiles::resolve(&settings).unwrap();
    for value in [&youtube, &vimeo] {
        let errors = validate_document(value, ProfileName::Full, &off).unwrap_err();
        assert_eq!(errors.iter().next().unwrap().path, "blocks[0]");
    }
    for profile in ProfileName::ALL {
        assert!(!off.profile(*profile).features.contains(&Feature::Video));
    }
    assert!(off.video_providers().is_empty());
}

#[test]
fn the_effective_profiles_without_configuration_are_the_code_profiles() {
    let profiles = EffectiveProfiles::resolve(&EditorSettings::default()).unwrap();
    assert_eq!(profiles, EffectiveProfiles::unrestricted());
    assert_eq!(
        profiles
            .video_providers()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [VideoProvider::Youtube, VideoProvider::Vimeo]
    );
    let view = profiles.to_dto();
    let names: Vec<_> = view.profiles.iter().map(|p| p.name).collect();
    assert_eq!(names, ProfileName::ALL);
    assert_eq!(view.profiles[0].features, Feature::ALL);
    assert!(view.profiles[2].single_block);
    assert!(!view.profiles[0].single_block);
    assert_eq!(view.profiles[0].limits, limits());
    assert_eq!(limits().max_document_bytes, 1_048_576);
}

#[test]
fn a_configuration_that_does_not_only_narrow_is_refused_and_names_the_problem() {
    let resolve = |profile: &str, features: &[&str]| {
        let mut settings = EditorSettings::default();
        settings.profiles.insert(
            profile.to_owned(),
            ProfileSettings {
                features: features.iter().map(|f| (*f).to_owned()).collect(),
            },
        );
        EffectiveProfiles::resolve(&settings)
            .unwrap_err()
            .to_string()
    };
    assert!(resolve("wizard", &[]).contains("wizard"));
    assert!(resolve("full", &["telepathy"]).contains("telepathy"));
    // Widening: the option profile has no headings, the student profile no video.
    assert!(resolve("option", &["headings"]).contains("headings"));
    assert!(resolve("student", &["video"]).contains("video"));

    let settings = EditorSettings {
        video_providers: vec!["vimeo".to_owned(), "dailymotion".to_owned()],
        ..EditorSettings::default()
    };
    let error = EffectiveProfiles::resolve(&settings)
        .unwrap_err()
        .to_string();
    assert!(error.contains("dailymotion"), "{error}");
}
