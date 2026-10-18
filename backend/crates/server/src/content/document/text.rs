//! The text of a document, for search, word counts and the AI agent (ADR-0025).

use api_types::{Block, Document, Inline, LinkRun};

/// Blocks in order, one per line; a block's children right after it. Inline and link text as
/// they are; a formula, a diagram and a code block as their source; a table as rows separated by
/// `\n` with cells separated by `\t`. A divider and a video give nothing, not even a line.
pub fn plain_text(document: &Document) -> String {
    let mut lines = Vec::new();
    push_blocks(&document.blocks, &mut lines);
    lines.join("\n")
}

fn push_blocks(blocks: &[Block], lines: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Divider { .. } | Block::Video { .. } => {}
            Block::Code { props, .. } => lines.push(props.source.clone()),
            Block::Math { props, .. } | Block::Mermaid { props, .. } => {
                lines.push(props.source.clone());
            }
            Block::Table { props, .. } => lines.push(
                props
                    .rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| inline_text(cell))
                            .collect::<Vec<_>>()
                            .join("\t")
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Block::Paragraph { .. }
            | Block::Quote { .. }
            | Block::Heading { .. }
            | Block::BulletedListItem { .. }
            | Block::NumberedListItem { .. }
            | Block::CheckListItem { .. } => lines.push(inline_text(block.content())),
        }
        push_blocks(block.children(), lines);
    }
}

fn inline_text(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .map(|inline| match inline {
            Inline::Text(run) => run.text.clone(),
            Inline::Link { content, .. } => content
                .iter()
                .map(|LinkRun::Text(run)| run.text.as_str())
                .collect(),
            Inline::Math { source } => source.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests;
