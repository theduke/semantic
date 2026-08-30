use dioxus::prelude::*;
use serde_json::Value;

use crate::{
    component_spec::{is_safe_media_url, is_safe_url},
    document_v2::{
        COMPONENT_BLOCKQUOTE, COMPONENT_BULLET_LIST, COMPONENT_CODE_BLOCK, COMPONENT_HARD_BREAK,
        COMPONENT_HEADING_V2, COMPONENT_IMAGE, COMPONENT_LIST_ITEM_V2, COMPONENT_MENTION_V2,
        COMPONENT_OPAQUE_MARKDOWN_BLOCK, COMPONENT_OPAQUE_MARKDOWN_INLINE, COMPONENT_ORDERED_LIST,
        COMPONENT_PARAGRAPH_V2, COMPONENT_TABLE_CELL_V2, COMPONENT_TABLE_HEADER,
        COMPONENT_TABLE_V2, COMPONENT_TASK_ITEM, COMPONENT_TASK_LIST, COMPONENT_TEXT_V2,
        COMPONENT_THEMATIC_BREAK, ComponentDocumentV2, ComponentMark, ComponentNode,
    },
};

#[component]
pub(crate) fn ReadOnlyDocumentV2(document: ComponentDocumentV2) -> Element {
    rsx! {
        article { class: "dxeditor__document dxeditor__read-only",
            for node in document.root.content { ReadOnlyNodeV2 { node } }
        }
    }
}

#[component]
fn ReadOnlyNodeV2(node: ComponentNode) -> Element {
    match node.kind.0.as_str() {
        COMPONENT_PARAGRAPH_V2 => rsx! { p { InlineChildren { nodes: node.content } } },
        COMPONENT_HEADING_V2 => {
            let level = node.attrs.get("level").and_then(Value::as_u64).unwrap_or(1);
            match level {
                1 => rsx! { h1 { InlineChildren { nodes: node.content } } },
                2 => rsx! { h2 { InlineChildren { nodes: node.content } } },
                3 => rsx! { h3 { InlineChildren { nodes: node.content } } },
                4 => rsx! { h4 { InlineChildren { nodes: node.content } } },
                5 => rsx! { h5 { InlineChildren { nodes: node.content } } },
                _ => rsx! { h6 { InlineChildren { nodes: node.content } } },
            }
        }
        COMPONENT_BLOCKQUOTE => rsx! { blockquote { BlockChildren { nodes: node.content } } },
        COMPONENT_CODE_BLOCK => rsx! { pre { code { "{node.text_content()}" } } },
        COMPONENT_THEMATIC_BREAK => rsx! { hr {} },
        COMPONENT_BULLET_LIST => rsx! { ul { ListChildren { nodes: node.content } } },
        COMPONENT_ORDERED_LIST => {
            let start = node.attrs.get("start").and_then(Value::as_u64).unwrap_or(1) as i64;
            rsx! { ol { start, ListChildren { nodes: node.content } } }
        }
        COMPONENT_TASK_LIST => {
            rsx! { ul { class: "dxeditor__task-list", ListChildren { nodes: node.content } } }
        }
        COMPONENT_LIST_ITEM_V2 | COMPONENT_TASK_ITEM => {
            let checked = node.attrs.get("checked").and_then(Value::as_bool);
            rsx! { li {
                if let Some(checked) = checked {
                    input { r#type: "checkbox", checked, disabled: true, aria_label: if checked { "Completed task" } else { "Incomplete task" } }
                }
                BlockChildren { nodes: node.content }
            } }
        }
        COMPONENT_TABLE_V2 => rsx! { TableV2 { node } },
        COMPONENT_OPAQUE_MARKDOWN_BLOCK => {
            let source = node
                .attrs
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default();
            rsx! { pre { class: "dxeditor-engine__opaque", code { "{source}" } } }
        }
        COMPONENT_TEXT_V2
        | COMPONENT_HARD_BREAK
        | COMPONENT_IMAGE
        | COMPONENT_MENTION_V2
        | COMPONENT_OPAQUE_MARKDOWN_INLINE => rsx! { InlineNodeV2 { node } },
        _ => {
            let fallback = node
                .attrs
                .get("fallback")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| node.text_content());
            rsx! { span { class: "dxeditor__fallback", "{fallback}" } }
        }
    }
}

#[component]
fn BlockChildren(nodes: Vec<ComponentNode>) -> Element {
    rsx! { for node in nodes { ReadOnlyNodeV2 { node } } }
}

#[component]
fn ListChildren(nodes: Vec<ComponentNode>) -> Element {
    rsx! { for node in nodes { ReadOnlyNodeV2 { node } } }
}

#[component]
fn InlineChildren(nodes: Vec<ComponentNode>) -> Element {
    rsx! { for node in nodes { InlineNodeV2 { node } } }
}

#[component]
fn InlineNodeV2(node: ComponentNode) -> Element {
    match node.kind.0.as_str() {
        COMPONENT_TEXT_V2 => render_marked_text(node.text.unwrap_or_default(), &node.marks, 0),
        COMPONENT_HARD_BREAK => rsx! { br {} },
        COMPONENT_IMAGE => {
            let src = node
                .attrs
                .get("src")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let alt = node
                .attrs
                .get("alt")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title = node.attrs.get("title").and_then(Value::as_str);
            if is_safe_media_url(src) {
                rsx! { img { src, alt, title } }
            } else {
                rsx! { span { class: "dxeditor__fallback", "{alt}" } }
            }
        }
        COMPONENT_MENTION_V2 => {
            let label = node
                .attrs
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            rsx! { span { class: "dxeditor-engine__mention", "@{label}" } }
        }
        COMPONENT_OPAQUE_MARKDOWN_INLINE => {
            let source = node
                .attrs
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default();
            rsx! { code { class: "dxeditor-engine__opaque-inline", "{source}" } }
        }
        _ => {
            let fallback = node
                .attrs
                .get("fallback")
                .and_then(Value::as_str)
                .unwrap_or_default();
            rsx! { span { class: "dxeditor__fallback", "{fallback}" } }
        }
    }
}

fn render_marked_text(text: String, marks: &[ComponentMark], index: usize) -> Element {
    let Some(mark) = marks.get(index) else {
        return rsx! { "{text}" };
    };
    let child = render_marked_text(text, marks, index + 1);
    match mark.kind.0.as_str() {
        "bold" => rsx! { strong { {child} } },
        "italic" => rsx! { em { {child} } },
        "strike" => rsx! { del { {child} } },
        "code" => rsx! { code { {child} } },
        "link" => {
            let href = mark
                .attrs
                .get("href")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let title = mark.attrs.get("title").and_then(Value::as_str);
            if is_safe_url(href) {
                rsx! { a { href, title, rel: "noopener noreferrer", {child} } }
            } else {
                child
            }
        }
        _ => child,
    }
}

#[component]
fn TableV2(node: ComponentNode) -> Element {
    let mut rows = node.content;
    let header = rows.first().is_some_and(|row| {
        row.content
            .iter()
            .all(|cell| cell.kind.0 == COMPONENT_TABLE_HEADER)
    });
    let header_row = if header { Some(rows.remove(0)) } else { None };
    rsx! {
        div { class: "tableWrapper",
            table {
                if let Some(row) = header_row {
                    thead { TableRowV2 { row, header: true } }
                }
                tbody { for row in rows { TableRowV2 { row, header: false } } }
            }
        }
    }
}

#[component]
fn TableRowV2(row: ComponentNode, header: bool) -> Element {
    rsx! { tr {
        for cell in row.content {
            if header || cell.kind.0 == COMPONENT_TABLE_HEADER {
                th { scope: "col", BlockChildren { nodes: cell.content } }
            } else if cell.kind.0 == COMPONENT_TABLE_CELL_V2 {
                td { BlockChildren { nodes: cell.content } }
            }
        }
    } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown_v2::MarkdownDocumentFormat;
    use crate::{DecodeOptions, DocumentFormat, EditorCatalog, EditorPayload, FORMAT_MARKDOWN};
    use serde_json::json;

    #[test]
    fn readonly_v2_uses_semantic_tables_and_escapes_opaque_html() {
        let catalog = EditorCatalog::default();
        let document = MarkdownDocumentFormat::new()
            .decode(
                &EditorPayload::new(
                    FORMAT_MARKDOWN,
                    json!("| h |\n| --- |\n| x |\n\n<script>alert(1)</script>"),
                ),
                catalog.component_specs(),
                DecodeOptions::default(),
            )
            .unwrap()
            .document;
        let html = dioxus_ssr::render_element(rsx! { ReadOnlyDocumentV2 { document } });
        assert!(html.contains("<thead>"));
        assert!(html.contains("<th scope=\"col\""));
        assert!(!html.contains("<script>"));
        assert!(html.contains("&#60;script&#62;"), "{html}");
    }

    #[test]
    fn readonly_v2_renders_tasks_media_breaks_links_marks_and_mentions() {
        let catalog = EditorCatalog::default();
        let document = MarkdownDocumentFormat::new()
            .decode(
                &EditorPayload::new(
                    FORMAT_MARKDOWN,
                    json!("- [x] done\n\n![diagram](https://example.com/a.png)  \n[link](https://example.com) ~~old~~ [@Ada](semantic:entity:user-1)"),
                ),
                catalog.component_specs(),
                DecodeOptions::default(),
            )
            .unwrap()
            .document;
        let html = dioxus_ssr::render_element(rsx! { ReadOnlyDocumentV2 { document } });
        assert!(html.contains("type=\"checkbox\""), "{html}");
        assert!(html.contains("<img"), "{html}");
        assert!(html.contains("<br/>"), "{html}");
        assert!(html.contains("href=\"https://example.com\""), "{html}");
        assert!(html.contains("<del>old</del>"), "{html}");
        assert!(html.contains("@Ada"), "{html}");
    }
}
