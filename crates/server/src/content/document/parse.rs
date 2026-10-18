//! Turns a JSON value into a typed [`Document`], recording every problem with its path.
//!
//! Hand-written because a serde error carries no path (and `serde_path_to_error` is not in
//! ADR-0014). Messages are fixed text and paths hold only indices and fixed keys, so nothing
//! the client sent is echoed back. An unknown key is reported on its parent for the same reason.

use std::collections::HashSet;
use std::sync::LazyLock;

use api_types::{
    Block, CheckProps, CodeLanguage, CodeProps, Document, HeadingProps, Inline, LinkRun,
    SourceProps, Styles, TableProps, TextRun, VideoProps, VideoProvider,
};
use serde_json::{Map, Value};

use super::{
    DocumentError, MAX_CODE_BYTES, MAX_DEPTH, MAX_ERRORS, MAX_FORMULA_BYTES, MAX_HREF_CHARS,
    MAX_ID_CHARS, MAX_TABLE_COLUMNS, MAX_TABLE_ROWS,
};

type Object = Map<String, Value>;

static NO_PROPS: LazyLock<Object> = LazyLock::new(Map::new);

/// The typed document, or `None` when something was wrong; the problems are in the second part.
pub(super) fn parse_document(value: &Value) -> (Option<Document>, Vec<DocumentError>) {
    let mut walk = Walk {
        errors: Vec::new(),
        ids: HashSet::new(),
    };
    let document = walk.document(value);
    (document, walk.errors)
}

fn at(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn nth(path: &str, index: usize) -> String {
    format!("{path}[{index}]")
}

struct Walk {
    errors: Vec<DocumentError>,
    ids: HashSet<String>,
}

impl Walk {
    fn err(&mut self, path: &str, message: &'static str) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.push(DocumentError {
                path: path.to_owned(),
                message,
            });
        }
    }

    fn object<'a>(&mut self, value: &'a Value, path: &str) -> Option<&'a Object> {
        let object = value.as_object();
        if object.is_none() {
            self.err(path, "must be an object");
        }
        object
    }

    fn unknown_keys(&mut self, object: &Object, allowed: &[&str], path: &str) {
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            self.err(path, "has an unknown property");
        }
    }

    fn document(&mut self, value: &Value) -> Option<Document> {
        let object = self.object(value, "")?;
        self.unknown_keys(object, &["v", "blocks", "attachments"], "");
        if object.get("v").and_then(Value::as_u64) != Some(1) {
            self.err("v", "must be 1");
        }
        let blocks = match object.get("blocks") {
            Some(Value::Array(blocks)) => self.blocks(blocks, "blocks", 1),
            _ => {
                self.err("blocks", "must be an array");
                Vec::new()
            }
        };
        match object.get("attachments") {
            None => {}
            Some(Value::Array(files)) if files.is_empty() => {}
            Some(Value::Array(_)) => self.err("attachments", "must be empty for now"),
            Some(_) => self.err("attachments", "must be an array"),
        }
        Some(Document {
            v: 1,
            blocks,
            attachments: Vec::new(),
        })
    }

    fn blocks(&mut self, values: &[Value], path: &str, depth: usize) -> Vec<Block> {
        values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| self.block(value, &nth(path, index), depth))
            .collect()
    }

    fn id(&mut self, value: Option<&Value>, path: &str) -> String {
        let here = at(path, "id");
        let Some(Value::String(id)) = value else {
            self.err(&here, "must be a string");
            return String::new();
        };
        let length_ok = (1..=MAX_ID_CHARS).contains(&id.chars().count());
        if !length_ok
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            self.err(
                &here,
                "must be 1 to 64 characters from A-Z, a-z, 0-9, - and _",
            );
            return String::new();
        }
        if !self.ids.insert(id.clone()) {
            self.err(&here, "must be unique in the document");
        }
        id.clone()
    }

    /// The `props` object, checked for unknown keys. A block that has none gets an empty one.
    fn props<'a>(&mut self, object: &'a Object, path: &str, allowed: &[&str]) -> &'a Object {
        let here = at(path, "props");
        match object.get("props") {
            None => &NO_PROPS,
            Some(Value::Object(props)) => {
                self.unknown_keys(props, allowed, &here);
                props
            }
            Some(_) => {
                self.err(&here, "must be an object");
                &NO_PROPS
            }
        }
    }

    fn string_field(&mut self, map: &Object, key: &str, path: &str, max_bytes: usize) -> String {
        match map.get(key) {
            Some(Value::String(text)) if has_control_character(text) => {
                self.err(&at(path, key), "contains a control character");
                String::new()
            }
            Some(Value::String(text)) if text.len() <= max_bytes => text.clone(),
            Some(Value::String(_)) => {
                self.err(&at(path, key), "is too long");
                String::new()
            }
            _ => {
                self.err(&at(path, key), "must be a string");
                String::new()
            }
        }
    }

    fn bool_field(&mut self, map: &Object, key: &str, path: &str) -> bool {
        match map.get(key) {
            Some(Value::Bool(value)) => *value,
            _ => {
                self.err(&at(path, key), "must be true or false");
                false
            }
        }
    }

    fn inlines(&mut self, value: Option<&Value>, path: &str) -> Vec<Inline> {
        match value {
            None => Vec::new(),
            Some(Value::Array(runs)) => runs
                .iter()
                .enumerate()
                .filter_map(|(index, run)| self.inline(run, &nth(path, index)))
                .collect(),
            Some(_) => {
                self.err(path, "must be an array");
                Vec::new()
            }
        }
    }

    /// `content` of a block that holds no inline text: absent or empty.
    fn no_content(&mut self, object: &Object, path: &str) {
        match object.get("content") {
            None => {}
            Some(Value::Array(runs)) if runs.is_empty() => {}
            Some(_) => self.err(&at(path, "content"), "is not allowed on this block"),
        }
    }

    fn content(&mut self, object: &Object, path: &str) -> Vec<Inline> {
        self.inlines(object.get("content"), &at(path, "content"))
    }

    /// `children` of a block that cannot nest: absent or empty.
    fn no_children(&mut self, object: &Object, path: &str) {
        match object.get("children") {
            None => {}
            Some(Value::Array(children)) if children.is_empty() => {}
            Some(_) => self.err(&at(path, "children"), "is not allowed on this block"),
        }
    }

    fn children(&mut self, object: &Object, path: &str, depth: usize) -> Vec<Block> {
        let here = at(path, "children");
        match object.get("children") {
            None => Vec::new(),
            Some(Value::Array(children)) => self.blocks(children, &here, depth + 1),
            Some(_) => {
                self.err(&here, "must be an array");
                Vec::new()
            }
        }
    }

    fn block(&mut self, value: &Value, path: &str, depth: usize) -> Option<Block> {
        let object = self.object(value, path)?;
        if depth > MAX_DEPTH {
            self.err(path, "is nested too deeply");
            return None;
        }
        self.unknown_keys(
            object,
            &["id", "type", "props", "content", "children"],
            path,
        );
        let id = self.id(object.get("id"), path);
        let Some(Value::String(kind)) = object.get("type") else {
            self.err(&at(path, "type"), "must be a string");
            return None;
        };
        let ppath = at(path, "props");
        let block = match kind.as_str() {
            "paragraph" | "quote" => {
                self.props(object, path, &[]);
                let content = self.content(object, path);
                self.no_children(object, path);
                if kind == "paragraph" {
                    Block::Paragraph { id, content }
                } else {
                    Block::Quote { id, content }
                }
            }
            "heading" => {
                let props = self.props(object, path, &["level"]);
                let level = match props.get("level").and_then(Value::as_u64) {
                    Some(level @ 1..=3) => u8::try_from(level).unwrap_or(1),
                    _ => {
                        self.err(&at(&ppath, "level"), "must be 1, 2 or 3");
                        1
                    }
                };
                let content = self.content(object, path);
                self.no_children(object, path);
                Block::Heading {
                    id,
                    props: HeadingProps { level },
                    content,
                }
            }
            "bulleted_list_item" | "numbered_list_item" => {
                self.props(object, path, &[]);
                let content = self.content(object, path);
                let children = self.children(object, path, depth);
                if kind == "bulleted_list_item" {
                    Block::BulletedListItem {
                        id,
                        content,
                        children,
                    }
                } else {
                    Block::NumberedListItem {
                        id,
                        content,
                        children,
                    }
                }
            }
            "check_list_item" => {
                let props = self.props(object, path, &["checked"]);
                let checked = self.bool_field(props, "checked", &ppath);
                let content = self.content(object, path);
                let children = self.children(object, path, depth);
                Block::CheckListItem {
                    id,
                    props: CheckProps { checked },
                    content,
                    children,
                }
            }
            "divider" => {
                self.props(object, path, &[]);
                self.no_content(object, path);
                self.no_children(object, path);
                Block::Divider { id }
            }
            "table" => {
                let props = self.props(object, path, &["header_row", "rows"]);
                let header_row = self.bool_field(props, "header_row", &ppath);
                let rows = self.table_rows(props, &ppath);
                self.no_content(object, path);
                self.no_children(object, path);
                Block::Table {
                    id,
                    props: TableProps { header_row, rows },
                }
            }
            "code" => {
                let props = self.props(object, path, &["language", "source"]);
                let language = match props.get("language") {
                    // The one silent rewrite: a language outside the list is stored as text.
                    Some(Value::String(name)) => {
                        CodeLanguage::from_name(name).unwrap_or(CodeLanguage::Text)
                    }
                    _ => {
                        self.err(&at(&ppath, "language"), "must be a string");
                        CodeLanguage::Text
                    }
                };
                let source = self.string_field(props, "source", &ppath, MAX_CODE_BYTES);
                self.no_content(object, path);
                self.no_children(object, path);
                Block::Code {
                    id,
                    props: CodeProps { language, source },
                }
            }
            "math" | "mermaid" => {
                let props = self.props(object, path, &["source"]);
                let source = self.string_field(props, "source", &ppath, MAX_FORMULA_BYTES);
                self.no_content(object, path);
                self.no_children(object, path);
                let props = SourceProps { source };
                if kind == "math" {
                    Block::Math { id, props }
                } else {
                    Block::Mermaid { id, props }
                }
            }
            "video" => {
                let props = self.props(object, path, &["provider", "video_id"]);
                let provider = match props.get("provider") {
                    Some(Value::String(name)) => VideoProvider::from_name(name),
                    _ => None,
                };
                if provider.is_none() {
                    self.err(&at(&ppath, "provider"), "must be youtube or vimeo");
                }
                let video_id = self.string_field(props, "video_id", &ppath, 64);
                if let Some(provider) = provider
                    && props.get("video_id").is_some_and(Value::is_string)
                    && !valid_video_id(provider, &video_id)
                {
                    self.err(
                        &at(&ppath, "video_id"),
                        "is not a valid id for this provider",
                    );
                }
                self.no_content(object, path);
                self.no_children(object, path);
                Block::Video {
                    id,
                    props: VideoProps {
                        provider: provider.unwrap_or(VideoProvider::Youtube),
                        video_id,
                    },
                }
            }
            "image" | "html" => {
                self.err(&at(path, "type"), "is not available yet");
                return None;
            }
            _ => {
                self.err(&at(path, "type"), "is not a known block type");
                return None;
            }
        };
        Some(block)
    }

    fn table_rows(&mut self, props: &Object, ppath: &str) -> Vec<Vec<Vec<Inline>>> {
        let here = at(ppath, "rows");
        let Some(Value::Array(rows)) = props.get("rows") else {
            self.err(&here, "must be an array");
            return Vec::new();
        };
        if rows.len() > MAX_TABLE_ROWS {
            self.err(&here, "has too many rows");
            return Vec::new();
        }
        let mut width = None;
        let mut parsed = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let row_path = nth(&here, index);
            let Value::Array(cells) = row else {
                self.err(&row_path, "must be an array");
                continue;
            };
            if cells.len() > MAX_TABLE_COLUMNS {
                self.err(&row_path, "has too many columns");
                continue;
            }
            match width {
                None => width = Some(cells.len()),
                Some(first) if first != cells.len() => {
                    self.err(&row_path, "must have as many cells as the first row");
                }
                Some(_) => {}
            }
            parsed.push(
                cells
                    .iter()
                    .enumerate()
                    .map(|(column, cell)| self.inlines(Some(cell), &nth(&row_path, column)))
                    .collect(),
            );
        }
        parsed
    }

    fn inline(&mut self, value: &Value, path: &str) -> Option<Inline> {
        let object = self.object(value, path)?;
        match object.get("type").and_then(Value::as_str) {
            Some("text") => {
                self.unknown_keys(object, &["type", "text", "styles"], path);
                Some(Inline::Text(self.text_run(object, path)))
            }
            Some("link") => {
                self.unknown_keys(object, &["type", "href", "content"], path);
                let href = match object.get("href") {
                    Some(Value::String(href)) if valid_href(href) => href.clone(),
                    _ => {
                        self.err(
                            &at(path, "href"),
                            "must be an http or https address of at most 2048 characters",
                        );
                        String::new()
                    }
                };
                let content = self.link_runs(object.get("content"), &at(path, "content"));
                Some(Inline::Link { href, content })
            }
            Some("math") => {
                self.unknown_keys(object, &["type", "source"], path);
                let source = self.string_field(object, "source", path, MAX_FORMULA_BYTES);
                Some(Inline::Math { source })
            }
            _ => {
                self.err(&at(path, "type"), "is not a known inline type");
                None
            }
        }
    }

    fn link_runs(&mut self, value: Option<&Value>, path: &str) -> Vec<LinkRun> {
        let Some(Value::Array(runs)) = value else {
            self.err(path, "must be an array");
            return Vec::new();
        };
        let mut parsed = Vec::with_capacity(runs.len());
        for (index, run) in runs.iter().enumerate() {
            let run_path = nth(path, index);
            let Some(object) = self.object(run, &run_path) else {
                continue;
            };
            if object.get("type").and_then(Value::as_str) != Some("text") {
                self.err(&at(&run_path, "type"), "must be text");
                continue;
            }
            self.unknown_keys(object, &["type", "text", "styles"], &run_path);
            parsed.push(LinkRun::Text(self.text_run(object, &run_path)));
        }
        parsed
    }

    fn text_run(&mut self, object: &Object, path: &str) -> TextRun {
        let text = self.string_field(object, "text", path, usize::MAX);
        let styles = match object.get("styles") {
            None => Styles::default(),
            Some(Value::Object(styles)) => {
                let here = at(path, "styles");
                self.unknown_keys(styles, &["bold", "italic", "strike", "code"], &here);
                let flag = |walk: &mut Self, key: &str| match styles.get(key) {
                    None => false,
                    Some(Value::Bool(value)) => *value,
                    Some(_) => {
                        walk.err(&at(&here, key), "must be true or false");
                        false
                    }
                };
                Styles {
                    bold: flag(self, "bold"),
                    italic: flag(self, "italic"),
                    strike: flag(self, "strike"),
                    code: flag(self, "code"),
                }
            }
            Some(_) => {
                self.err(&at(path, "styles"), "must be an object");
                Styles::default()
            }
        };
        TextRun { text, styles }
    }
}

/// A control character other than a line break or a tab. The database cannot store a NUL in text,
/// and no other control character belongs in a document either (ADR-0039).
fn has_control_character(text: &str) -> bool {
    text.chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
}

/// http or https, something after the scheme, no whitespace or control characters.
pub(crate) fn valid_href(href: &str) -> bool {
    if href.chars().count() > MAX_HREF_CHARS
        || href.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return false;
    }
    let lower = href.to_ascii_lowercase();
    ["http://", "https://"].iter().any(|scheme| {
        lower
            .strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty())
    })
}

fn valid_video_id(provider: VideoProvider, id: &str) -> bool {
    match provider {
        VideoProvider::Youtube => {
            id.len() == 11
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        }
        VideoProvider::Vimeo => {
            (1..=12).contains(&id.len()) && id.chars().all(|c| c.is_ascii_digit())
        }
    }
}
