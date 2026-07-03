#[cfg(feature = "markdown")]
use serde_json::{Map, Value};

#[cfg(feature = "markdown")]
use crate::{
    EditorError,
    codec::{DecodeContext, EditorCodec, EditorPayload, EncodeContext},
    document::{
        BlockNode, COMPONENT_CODE, COMPONENT_DIVIDER, COMPONENT_HEADING, COMPONENT_LINK,
        COMPONENT_MENTION, COMPONENT_PARAGRAPH, COMPONENT_QUOTE, EditorDocument, InlineNode, Mark,
        NodeContent,
    },
};

#[cfg(feature = "markdown")]
pub struct MarkdownCodec;

#[cfg(feature = "markdown")]
impl EditorCodec for MarkdownCodec {
    fn format(&self) -> &str {
        "markdown"
    }

    fn decode(
        &self,
        payload: &EditorPayload,
        _ctx: DecodeContext,
    ) -> Result<EditorDocument, EditorError> {
        let Some(markdown) = payload.value.as_str() else {
            return Err(EditorError::InvalidPayload {
                format: payload.format.clone(),
                message: "expected string".to_string(),
            });
        };
        Ok(parse_markdown(markdown))
    }

    fn encode(
        &self,
        document: &EditorDocument,
        _ctx: EncodeContext,
    ) -> Result<EditorPayload, EditorError> {
        Ok(EditorPayload::new(
            "markdown",
            Value::String(serialize_markdown(document)),
        ))
    }
}

#[cfg(feature = "markdown")]
pub fn parse_markdown(markdown: &str) -> EditorDocument {
    let mut blocks = Vec::new();
    let mut paragraph = Vec::<String>::new();
    let mut lines = markdown.lines().peekable();
    let mut block_index = 1usize;

    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph, &mut block_index);
            continue;
        }

        if line.trim() == "---" {
            flush_paragraph(&mut blocks, &mut paragraph, &mut block_index);
            blocks.push(BlockNode::new(
                format!("block-{block_index}"),
                COMPONENT_DIVIDER,
                Map::new(),
                NodeContent::Void,
            ));
            block_index += 1;
            continue;
        }

        if line.starts_with("```") {
            flush_paragraph(&mut blocks, &mut paragraph, &mut block_index);
            let language = line.trim_start_matches("```").trim();
            let mut text = String::new();
            for code_line in lines.by_ref() {
                if code_line.starts_with("```") {
                    break;
                }
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(code_line);
            }
            let mut attrs = Map::new();
            if !language.is_empty() {
                attrs.insert("language".to_string(), Value::String(language.to_string()));
            }
            blocks.push(BlockNode::new(
                format!("block-{block_index}"),
                COMPONENT_CODE,
                attrs,
                NodeContent::Inline(vec![InlineNode::text(
                    format!("text-{block_index}-1"),
                    text,
                )]),
            ));
            block_index += 1;
            continue;
        }

        if let Some((level, text)) = heading_line(line) {
            flush_paragraph(&mut blocks, &mut paragraph, &mut block_index);
            blocks.push(BlockNode::heading(
                format!("block-{block_index}"),
                level,
                parse_inline(text, block_index),
            ));
            block_index += 1;
            continue;
        }

        if let Some(text) = line.strip_prefix("> ") {
            flush_paragraph(&mut blocks, &mut paragraph, &mut block_index);
            blocks.push(BlockNode::new(
                format!("block-{block_index}"),
                COMPONENT_QUOTE,
                Map::new(),
                NodeContent::Inline(parse_inline(text, block_index)),
            ));
            block_index += 1;
            continue;
        }

        paragraph.push(line.to_string());
    }

    flush_paragraph(&mut blocks, &mut paragraph, &mut block_index);
    EditorDocument::new(blocks)
}

#[cfg(feature = "markdown")]
pub fn serialize_markdown(document: &EditorDocument) -> String {
    document
        .blocks
        .iter()
        .map(serialize_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(feature = "markdown")]
fn heading_line(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = line.get(hashes..)?;
    rest.strip_prefix(' ').map(|text| (hashes as u8, text))
}

#[cfg(feature = "markdown")]
fn flush_paragraph(
    blocks: &mut Vec<BlockNode>,
    paragraph: &mut Vec<String>,
    block_index: &mut usize,
) {
    if paragraph.is_empty() {
        return;
    }
    let text = paragraph.join("\n");
    blocks.push(BlockNode::paragraph(
        format!("block-{block_index}"),
        parse_inline(&text, *block_index),
    ));
    *block_index += 1;
    paragraph.clear();
}

#[cfg(feature = "markdown")]
fn parse_inline(text: &str, block_index: usize) -> Vec<InlineNode> {
    let mut nodes = Vec::new();
    let mut remaining = text;
    let mut inline_index = 1usize;

    while let Some(start) = remaining.find("[@") {
        let before = &remaining[..start];
        if !before.is_empty() {
            nodes.push(InlineNode::text(
                format!("text-{block_index}-{inline_index}"),
                before,
            ));
            inline_index += 1;
        }

        let mention_start = &remaining[start..];
        let Some(label_end) = mention_start.find("](") else {
            nodes.push(InlineNode::text(
                format!("text-{block_index}-{inline_index}"),
                mention_start,
            ));
            return nodes;
        };
        let label = &mention_start[2..label_end];
        let after_label = &mention_start[(label_end + 2)..];
        let Some(url_end) = after_label.find(')') else {
            nodes.push(InlineNode::text(
                format!("text-{block_index}-{inline_index}"),
                mention_start,
            ));
            return nodes;
        };
        let href = &after_label[..url_end];
        if let Some(entity_id) = href.strip_prefix("semantic:entity:") {
            nodes.push(InlineNode::mention(
                format!("mention-{block_index}-{inline_index}"),
                entity_id,
                label,
            ));
        } else {
            nodes.push(
                InlineNode::text(format!("text-{block_index}-{inline_index}"), label)
                    .with_mark(Mark::link(href)),
            );
        }
        inline_index += 1;
        remaining = &after_label[(url_end + 1)..];
    }

    if !remaining.is_empty() {
        nodes.push(InlineNode::text(
            format!("text-{block_index}-{inline_index}"),
            remaining,
        ));
    }
    nodes
}

#[cfg(feature = "markdown")]
fn serialize_block(block: &BlockNode) -> String {
    match block.component.as_str() {
        COMPONENT_HEADING => {
            let level = block
                .attrs
                .get("level")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 6) as usize;
            format!(
                "{} {}",
                "#".repeat(level),
                serialize_inline_content(&block.content)
            )
        }
        COMPONENT_QUOTE => format!("> {}", serialize_inline_content(&block.content)),
        COMPONENT_CODE => {
            let language = block
                .attrs
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or_default();
            format!(
                "```{language}\n{}\n```",
                serialize_inline_content(&block.content)
            )
        }
        COMPONENT_DIVIDER => "---".to_string(),
        COMPONENT_PARAGRAPH => serialize_inline_content(&block.content),
        _ => serialize_inline_content(&block.content),
    }
}

#[cfg(feature = "markdown")]
fn serialize_inline_content(content: &NodeContent) -> String {
    match content {
        NodeContent::Inline(inline) => inline.iter().map(serialize_inline).collect(),
        NodeContent::Blocks(blocks) => blocks
            .iter()
            .map(serialize_block)
            .collect::<Vec<_>>()
            .join("\n\n"),
        NodeContent::Table(table) => table
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| {
                        cell.blocks
                            .iter()
                            .map(serialize_block)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        NodeContent::Void => String::new(),
        NodeContent::Custom(value) => value.to_string(),
    }
}

#[cfg(feature = "markdown")]
fn serialize_inline(inline: &InlineNode) -> String {
    if inline.component == COMPONENT_MENTION {
        let id = inline
            .attrs
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return format!("[@{}](semantic:entity:{id})", escape_text(&inline.text));
    }

    let mut text = escape_text(&inline.text);
    for mark in inline.marks.iter().rev() {
        match mark.component.as_str() {
            "bold" => text = format!("**{text}**"),
            "italic" => text = format!("*{text}*"),
            "code" => text = format!("`{text}`"),
            COMPONENT_LINK => {
                if let Some(href) = mark.attrs.get("href").and_then(Value::as_str) {
                    text = format!("[{text}]({href})");
                }
            }
            _ => {}
        }
    }
    text
}

#[cfg(feature = "markdown")]
fn escape_text(text: &str) -> String {
    text.replace('[', "\\[").replace(']', "\\]")
}
