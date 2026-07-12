use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value};

use crate::{
    EditorError,
    codec::{DecodeContext, EditorCodec, EditorPayload, EncodeContext},
    document::{
        BlockNode, COMPONENT_CODE, COMPONENT_DIVIDER, COMPONENT_HEADING, COMPONENT_LINK,
        COMPONENT_MENTION, COMPONENT_PARAGRAPH, COMPONENT_QUOTE, EditorDocument, InlineNode, Mark,
        NodeContent,
    },
};

pub struct MarkdownCodec;

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

#[derive(Default)]
struct MarkdownBuilder {
    blocks: Vec<BlockNode>,
    inline: Vec<InlineNode>,
    marks: Vec<Mark>,
    block_index: usize,
    quote_depth: usize,
    block_kind: Option<BlockKind>,
    in_code_block: bool,
    code_text: String,
    code_language: Option<String>,
}

enum BlockKind {
    Paragraph,
    Heading(u8),
}

impl MarkdownBuilder {
    fn new() -> Self {
        Self {
            block_index: 1,
            ..Self::default()
        }
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let inline_index = self.inline.len() + 1;
        let link = self
            .marks
            .iter()
            .find(|mark| mark.component == COMPONENT_LINK)
            .and_then(|mark| mark.attrs.get("href"))
            .and_then(Value::as_str);
        if let Some(entity_id) = link.and_then(|href| href.strip_prefix("semantic:entity:"))
            && let Some(label) = text.strip_prefix('@')
        {
            let mut node = InlineNode::mention(
                format!("mention-{}-{inline_index}", self.block_index),
                entity_id,
                label,
            );
            node.marks.extend(
                self.marks
                    .iter()
                    .filter(|mark| mark.component != COMPONENT_LINK)
                    .cloned(),
            );
            self.inline.push(node);
            return;
        }

        let mut node = InlineNode::text(format!("text-{}-{inline_index}", self.block_index), text);
        node.marks.clone_from(&self.marks);
        self.inline.push(node);
    }

    fn finish_inline_block(&mut self) {
        let Some(kind) = self.block_kind.take() else {
            return;
        };
        let content = std::mem::take(&mut self.inline);
        let id = format!("block-{}", self.block_index);
        let block = if self.quote_depth > 0 {
            BlockNode::new(
                id,
                COMPONENT_QUOTE,
                Map::new(),
                NodeContent::Inline(content),
            )
        } else {
            match kind {
                BlockKind::Paragraph => BlockNode::paragraph(id, content),
                BlockKind::Heading(level) => BlockNode::heading(id, level, content),
            }
        };
        self.blocks.push(block);
        self.block_index += 1;
    }

    fn finish_code_block(&mut self) {
        let mut attrs = Map::new();
        if let Some(language) = self.code_language.take().filter(|value| !value.is_empty()) {
            attrs.insert("language".to_string(), Value::String(language));
        }
        let text = self.code_text.strip_suffix('\n').unwrap_or(&self.code_text);
        self.blocks.push(BlockNode::new(
            format!("block-{}", self.block_index),
            COMPONENT_CODE,
            attrs,
            NodeContent::Inline(vec![InlineNode::text(
                format!("text-{}-1", self.block_index),
                text,
            )]),
        ));
        self.code_text.clear();
        self.block_index += 1;
    }
}

pub fn parse_markdown(markdown: &str) -> EditorDocument {
    let mut builder = MarkdownBuilder::new();
    for event in Parser::new_ext(markdown, Options::empty()) {
        match event {
            Event::Start(Tag::Paragraph) => builder.block_kind = Some(BlockKind::Paragraph),
            Event::End(TagEnd::Paragraph) => builder.finish_inline_block(),
            Event::Start(Tag::Heading { level, .. }) => {
                builder.block_kind = Some(BlockKind::Heading(heading_level(level)));
            }
            Event::End(TagEnd::Heading(_)) => builder.finish_inline_block(),
            Event::Start(Tag::BlockQuote(_)) => builder.quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => builder.quote_depth -= 1,
            Event::Start(Tag::CodeBlock(kind)) => {
                builder.in_code_block = true;
                builder.code_language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().map(str::to_owned)
                    }
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                builder.finish_code_block();
                builder.in_code_block = false;
            }
            Event::Start(Tag::Strong) => builder.marks.push(Mark::new("bold")),
            Event::End(TagEnd::Strong) => remove_mark(&mut builder.marks, "bold"),
            Event::Start(Tag::Emphasis) => builder.marks.push(Mark::new("italic")),
            Event::End(TagEnd::Emphasis) => remove_mark(&mut builder.marks, "italic"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                builder.marks.push(Mark::link(dest_url.into_string()));
            }
            Event::End(TagEnd::Link) => remove_mark(&mut builder.marks, COMPONENT_LINK),
            Event::Text(text) if builder.in_code_block => builder.code_text.push_str(&text),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                builder.push_text(&text);
            }
            Event::Code(text) => {
                builder.marks.push(Mark::new("code"));
                builder.push_text(&text);
                remove_mark(&mut builder.marks, "code");
            }
            Event::SoftBreak | Event::HardBreak => builder.push_text("\n"),
            Event::Rule => {
                builder.blocks.push(BlockNode::new(
                    format!("block-{}", builder.block_index),
                    COMPONENT_DIVIDER,
                    Map::new(),
                    NodeContent::Void,
                ));
                builder.block_index += 1;
            }
            _ => {}
        }
    }
    EditorDocument::new(builder.blocks)
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn remove_mark(marks: &mut Vec<Mark>, component: &str) {
    if let Some(index) = marks.iter().rposition(|mark| mark.component == component) {
        marks.remove(index);
    }
}

pub fn serialize_markdown(document: &EditorDocument) -> String {
    document
        .blocks
        .iter()
        .map(serialize_block)
        .collect::<Vec<_>>()
        .join("\n\n")
}

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
        COMPONENT_QUOTE => serialize_inline_content(&block.content)
            .lines()
            .map(|line| format!("> {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
        COMPONENT_CODE => {
            let language = block
                .attrs
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let content = raw_text_content(&block.content);
            let fence = if content.contains("```") {
                "````"
            } else {
                "```"
            };
            format!("{fence}{language}\n{content}\n{fence}")
        }
        COMPONENT_DIVIDER => "---".to_string(),
        COMPONENT_PARAGRAPH => serialize_inline_content(&block.content),
        _ => serialize_inline_content(&block.content),
    }
}

fn raw_text_content(content: &NodeContent) -> String {
    match content {
        NodeContent::Inline(inline) => inline.iter().map(|node| node.text.as_str()).collect(),
        _ => String::new(),
    }
}

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
        NodeContent::Void | NodeContent::Custom(_) => String::new(),
    }
}

fn serialize_inline(inline: &InlineNode) -> String {
    if inline.component == COMPONENT_MENTION {
        let id = inline
            .attrs
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return format!(
            "[@{}](semantic:entity:{})",
            escape_text(&inline.text),
            escape_destination(id)
        );
    }

    let has_code = inline.marks.iter().any(|mark| mark.component == "code");
    let mut text = if has_code {
        code_span(&inline.text)
    } else {
        escape_text(&inline.text)
    };
    for mark in inline.marks.iter().rev() {
        match mark.component.as_str() {
            "bold" => text = format!("**{text}**"),
            "italic" => text = format!("*{text}*"),
            "code" => {}
            COMPONENT_LINK => {
                if let Some(href) = mark.attrs.get("href").and_then(Value::as_str) {
                    text = format!("[{text}]({})", escape_destination(href));
                }
            }
            _ => {}
        }
    }
    text
}

fn code_span(text: &str) -> String {
    let fence = if text.contains('`') { "``" } else { "`" };
    let padding = if text.starts_with('`') || text.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{padding}{text}{padding}{fence}")
}

fn escape_text(text: &str) -> String {
    text.chars()
        .flat_map(|ch| match ch {
            '\\' | '*' | '_' | '`' | '[' | ']' | '#' | '+' | '-' => {
                ['\\', ch].into_iter().collect::<Vec<_>>()
            }
            _ => vec![ch],
        })
        .collect()
}

fn escape_destination(destination: &str) -> String {
    destination.replace('\\', "\\\\").replace(')', "\\)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_markdown_is_encode_decode_idempotent() {
        let markdown = "# Heading *with emphasis*\n\nA **bold** paragraph with `code` and [a link](https://example.com).\ncontinued here\n\n> quoted **text**\n\n```rust\nlet value = `raw`;\n```\n\n---";
        let encoded = serialize_markdown(&parse_markdown(markdown));
        assert_eq!(serialize_markdown(&parse_markdown(&encoded)), encoded);
    }

    #[test]
    fn text_metacharacters_round_trip_as_text() {
        let samples = [
            "*literal*",
            "_literal_",
            "`literal`",
            "\\path",
            "# heading",
            "- item",
            "+ item",
            "[label]",
        ];
        for sample in samples {
            let document = EditorDocument::plain_text(sample);
            let encoded = serialize_markdown(&document);
            assert_eq!(
                parse_markdown(&encoded).text_content(),
                sample,
                "encoded as {encoded:?}"
            );
        }
    }

    #[test]
    fn groups_multiline_paragraph_and_quote_blocks() {
        let document = parse_markdown("first\nsecond\n\n> one\n> two");
        assert_eq!(document.blocks.len(), 2);
        assert_eq!(document.blocks[0].text_content(), "first\nsecond");
        assert_eq!(document.blocks[1].component, COMPONENT_QUOTE);
        assert_eq!(document.blocks[1].text_content(), "one\ntwo");
    }
}
