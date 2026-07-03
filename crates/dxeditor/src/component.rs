use std::collections::BTreeMap;

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::{
    EditorError,
    catalog::EditorCatalog,
    codec::EditorPayload,
    document::{
        BlockNode, COMPONENT_CODE, COMPONENT_DIVIDER, COMPONENT_HEADING, COMPONENT_LINK,
        COMPONENT_MENTION, COMPONENT_PARAGRAPH, COMPONENT_QUOTE, DOCUMENT_SCHEMA_V1,
        EditorDocument, InlineNode, Mark, NodeId,
    },
    selection::EditorSelection,
    selection_bridge,
    state::EditorState,
};

const DXEDITOR_STYLE: &str = r#"
.dxeditor {
  display: grid;
  width: min(100%, 980px);
  overflow: hidden;
  border: 1px solid #cfd8e3;
  border-radius: 8px;
  background: #ffffff;
  color: #17202a;
}

.dxeditor__toolbar,
.dxeditor__status {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding: 8px;
  background: #f6f8fa;
}

.dxeditor__toolbar {
  border-bottom: 1px solid #dfe5ec;
}

.dxeditor__toolbar-group {
  display: inline-flex;
  flex-wrap: wrap;
  gap: 4px;
  padding-right: 8px;
  border-right: 1px solid #dfe5ec;
}

.dxeditor__toolbar-group:last-child {
  border-right: 0;
}

.dxeditor__button {
  min-width: 32px;
  min-height: 30px;
  border: 1px solid #cfd8e3;
  border-radius: 6px;
  background: #ffffff;
  color: #17202a;
  padding: 4px 8px;
  font: inherit;
  font-size: 12px;
  font-weight: 650;
  line-height: 1;
  cursor: pointer;
}

.dxeditor__button:hover {
  border-color: #176b87;
  color: #176b87;
}

.dxeditor__button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}

.dxeditor__document {
  display: grid;
  gap: 2px;
  min-height: 260px;
  padding: 22px 28px;
  background: #ffffff;
}

.dxeditor__block {
  min-width: 0;
  margin: 0;
  border: 0;
  border-radius: 4px;
  padding: 2px 0;
  outline: none;
  overflow-wrap: anywhere;
  line-height: 1.6;
  white-space: pre-wrap;
}

.dxeditor__block:hover {
  background: transparent;
}

.dxeditor__block:focus {
  box-shadow: inset 3px 0 0 #176b87;
  padding-left: 8px;
}

.dxeditor__block[data-component="heading"] {
  font-weight: 720;
  line-height: 1.25;
}

.dxeditor__block[data-level="1"] {
  font-size: 26px;
}

.dxeditor__block[data-level="2"] {
  font-size: 21px;
}

.dxeditor__block[data-component="quote"] {
  border-left: 3px solid #9bb5c1;
  border-radius: 0 4px 4px 0;
  color: #435367;
  padding-left: 12px;
}

.dxeditor__block[data-component="code"] {
  background: #f6f8fa;
  border-radius: 4px;
  padding: 8px 10px;
  font-family: ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace;
  font-size: 13px;
}

.dxeditor__inline-code {
  border-radius: 4px;
  background: #eef2f6;
  padding: 0 4px;
  font-family: ui-monospace, SFMono-Regular, Consolas, "Liberation Mono", Menlo, monospace;
  font-size: 0.92em;
}

.dxeditor__mention {
  border-radius: 4px;
  background: #e9f2f5;
  color: #176b87;
  padding: 0 4px;
}

.dxeditor__divider {
  height: 1px;
  margin: 10px 0;
  border: 0;
  background: #cfd8e3;
}

.dxeditor__status {
  justify-content: space-between;
  border-top: 1px solid #dfe5ec;
  color: #617084;
  font-size: 12px;
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorComponentKind {
    Block,
    Inline,
    Mark,
    Table,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorComponentRegistration {
    pub id: String,
    pub label: String,
    pub kind: EditorComponentKind,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ComponentRegistry {
    components: BTreeMap<String, EditorComponentRegistration>,
}

impl ComponentRegistry {
    pub fn register(&mut self, component: EditorComponentRegistration) {
        self.components.insert(component.id.clone(), component);
    }

    pub fn component(&self, id: &str) -> Option<&EditorComponentRegistration> {
        self.components.get(id)
    }

    pub fn components(&self) -> impl Iterator<Item = &EditorComponentRegistration> {
        self.components.values()
    }
}

#[component]
pub fn Editor(
    value: EditorPayload,
    output_format: String,
    catalog: EditorCatalog,
    on_change: EventHandler<EditorPayload>,
    #[props(default)] readonly: bool,
) -> Element {
    let initial_document = catalog
        .codecs()
        .decode(&value)
        .unwrap_or_else(|_| EditorDocument::plain_text(""));
    let state = use_signal(move || {
        if readonly {
            EditorState::readonly(initial_document)
        } else {
            EditorState::new(initial_document)
        }
    });
    let render_version = use_signal(|| 0_u64);
    let _render_version = render_version();
    let editor_state = state.read().clone();
    let document = editor_state.document();
    let block_count = document.blocks.len();
    let word_count = document_word_count(&document);

    rsx! {
        style { {DXEDITOR_STYLE} }
        div { class: "dxeditor", "data-readonly": "{readonly}",
            if !readonly {
                EditorToolbar {
                    catalog: catalog.clone(),
                    editor_state: editor_state.clone(),
                    output_format: output_format.clone(),
                    on_change,
                    render_version,
                }
            }
            div { class: "dxeditor__document", role: "textbox", aria_multiline: "true",
                for block in document.blocks {
                    EditableBlock {
                        block,
                        catalog: catalog.clone(),
                        editor_state: editor_state.clone(),
                        output_format: output_format.clone(),
                        on_change,
                        readonly,
                        render_version,
                    }
                }
            }
            div { class: "dxeditor__status",
                span { "{block_count} blocks" }
                span { "{word_count} words" }
            }
        }
    }
}

#[component]
fn EditorToolbar(
    catalog: EditorCatalog,
    editor_state: EditorState,
    output_format: String,
    on_change: EventHandler<EditorPayload>,
    mut render_version: Signal<u64>,
) -> Element {
    let paragraph_catalog = catalog.clone();
    let paragraph_state = editor_state.clone();
    let paragraph_format = output_format.clone();

    let h1_catalog = catalog.clone();
    let h1_state = editor_state.clone();
    let h1_format = output_format.clone();

    let h2_catalog = catalog.clone();
    let h2_state = editor_state.clone();
    let h2_format = output_format.clone();

    let quote_catalog = catalog.clone();
    let quote_state = editor_state.clone();
    let quote_format = output_format.clone();

    let code_catalog = catalog.clone();
    let code_state = editor_state.clone();
    let code_format = output_format.clone();

    let bold_catalog = catalog.clone();
    let bold_state = editor_state.clone();
    let bold_format = output_format.clone();

    let italic_catalog = catalog.clone();
    let italic_state = editor_state.clone();
    let italic_format = output_format.clone();

    let inline_code_catalog = catalog.clone();
    let inline_code_state = editor_state.clone();
    let inline_code_format = output_format.clone();

    let insert_catalog = catalog.clone();
    let insert_state = editor_state.clone();
    let insert_format = output_format.clone();

    rsx! {
        div { class: "dxeditor__toolbar",
            div { class: "dxeditor__toolbar-group", role: "group", aria_label: "Block type",
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Paragraph",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        spawn_block_type_command(
                            paragraph_catalog.clone(),
                            paragraph_state.clone(),
                            COMPONENT_PARAGRAPH,
                            None,
                            paragraph_format.clone(),
                            on_change,
                            render_version,
                        );
                    },
                    "P"
                }
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Heading 1",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        spawn_block_type_command(
                            h1_catalog.clone(),
                            h1_state.clone(),
                            COMPONENT_HEADING,
                            Some(json!({ "level": 1 })),
                            h1_format.clone(),
                            on_change,
                            render_version,
                        );
                    },
                    "H1"
                }
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Heading 2",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        spawn_block_type_command(
                            h2_catalog.clone(),
                            h2_state.clone(),
                            COMPONENT_HEADING,
                            Some(json!({ "level": 2 })),
                            h2_format.clone(),
                            on_change,
                            render_version,
                        );
                    },
                    "H2"
                }
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Quote",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        spawn_block_type_command(
                            quote_catalog.clone(),
                            quote_state.clone(),
                            COMPONENT_QUOTE,
                            None,
                            quote_format.clone(),
                            on_change,
                            render_version,
                        );
                    },
                    "Quote"
                }
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Code block",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        spawn_block_type_command(
                            code_catalog.clone(),
                            code_state.clone(),
                            COMPONENT_CODE,
                            None,
                            code_format.clone(),
                            on_change,
                            render_version,
                        );
                    },
                    "Code"
                }
            }
            div { class: "dxeditor__toolbar-group", role: "group", aria_label: "Inline formatting",
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Bold",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        spawn_toggle_mark_command(
                            bold_catalog.clone(),
                            bold_state.clone(),
                            "bold",
                            bold_format.clone(),
                            on_change,
                            render_version,
                        );
                    },
                    "B"
                }
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Italic",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        spawn_toggle_mark_command(
                            italic_catalog.clone(),
                            italic_state.clone(),
                            "italic",
                            italic_format.clone(),
                            on_change,
                            render_version,
                        );
                    },
                    "I"
                }
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Inline code",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        spawn_toggle_mark_command(
                            inline_code_catalog.clone(),
                            inline_code_state.clone(),
                            "code",
                            inline_code_format.clone(),
                            on_change,
                            render_version,
                        );
                    },
                    "`"
                }
            }
            div { class: "dxeditor__toolbar-group", role: "group", aria_label: "Insert",
                button {
                    class: "dxeditor__button",
                    r#type: "button",
                    title: "Insert paragraph block",
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        apply_editor_command(
                            &insert_catalog,
                            &insert_state,
                            "editor.insert_block",
                            json!({ "component": COMPONENT_PARAGRAPH }),
                            &insert_format,
                            on_change,
                            &mut render_version,
                        );
                    },
                    "+ Block"
                }
            }
        }
    }
}

#[component]
fn EditableBlock(
    block: BlockNode,
    catalog: EditorCatalog,
    editor_state: EditorState,
    output_format: String,
    on_change: EventHandler<EditorPayload>,
    readonly: bool,
    mut render_version: Signal<u64>,
) -> Element {
    let text = block.text_content();
    let block_id = block.id.clone();
    let block_id_attr = block_id.0.clone();
    let text_len = text.chars().count();
    let inline = block_inline_segments(&block);
    let level = block
        .attrs
        .get("level")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 6);
    let contenteditable = if readonly { "false" } else { "true" };

    match block.component.as_str() {
        COMPONENT_HEADING => {
            let input_catalog = catalog.clone();
            let input_state = editor_state.clone();
            let input_block_id = block_id.clone();
            let input_format = output_format.clone();
            let mut input_render_version = render_version;
            rsx! {
                div {
                    class: "dxeditor__block",
                    "data-block-id": "{block_id_attr}",
                    "data-component": COMPONENT_HEADING,
                    "data-level": "{level}",
                    "data-text-len": "{text_len}",
                    contenteditable,
                    oninput: move |event: FormEvent| {
                        apply_block_text_edit(
                            &input_catalog,
                            &input_state,
                            input_block_id.clone(),
                            event.value(),
                            &input_format,
                            on_change,
                            &mut input_render_version,
                        );
                    },
                    onkeydown: block_keydown_handler(
                        catalog,
                        editor_state,
                        block_id,
                        text_len,
                        output_format,
                        on_change,
                        render_version,
                        readonly,
                    ),
                    for segment in inline {
                        InlineNodeView {
                            node: segment.node,
                            start: segment.start,
                            end: segment.end,
                        }
                    }
                }
            }
        }
        COMPONENT_QUOTE => {
            let input_catalog = catalog.clone();
            let input_state = editor_state.clone();
            let input_block_id = block_id.clone();
            let input_format = output_format.clone();
            let mut input_render_version = render_version;
            rsx! {
                div {
                    class: "dxeditor__block",
                    "data-block-id": "{block_id_attr}",
                    "data-component": COMPONENT_QUOTE,
                    "data-text-len": "{text_len}",
                    contenteditable,
                    oninput: move |event: FormEvent| {
                        apply_block_text_edit(
                            &input_catalog,
                            &input_state,
                            input_block_id.clone(),
                            event.value(),
                            &input_format,
                            on_change,
                            &mut input_render_version,
                        );
                    },
                    onkeydown: block_keydown_handler(
                        catalog,
                        editor_state,
                        block_id,
                        text_len,
                        output_format,
                        on_change,
                        render_version,
                        readonly,
                    ),
                    for segment in inline {
                        InlineNodeView {
                            node: segment.node,
                            start: segment.start,
                            end: segment.end,
                        }
                    }
                }
            }
        }
        COMPONENT_CODE => {
            let input_catalog = catalog.clone();
            let input_state = editor_state.clone();
            let input_block_id = block_id.clone();
            let input_format = output_format.clone();
            let mut input_render_version = render_version;
            rsx! {
                pre {
                    class: "dxeditor__block",
                    "data-block-id": "{block_id_attr}",
                    "data-component": COMPONENT_CODE,
                    "data-text-len": "{text_len}",
                    contenteditable,
                    oninput: move |event: FormEvent| {
                        apply_block_text_edit(
                            &input_catalog,
                            &input_state,
                            input_block_id.clone(),
                            event.value(),
                            &input_format,
                            on_change,
                            &mut input_render_version,
                        );
                    },
                    onkeydown: block_keydown_handler(
                        catalog,
                        editor_state,
                        block_id,
                        text_len,
                        output_format,
                        on_change,
                        render_version,
                        readonly,
                    ),
                    for segment in inline {
                        InlineNodeView {
                            node: segment.node,
                            start: segment.start,
                            end: segment.end,
                        }
                    }
                }
            }
        }
        COMPONENT_DIVIDER => rsx! {
            hr { class: "dxeditor__divider" }
        },
        _ => {
            let input_catalog = catalog.clone();
            let input_state = editor_state.clone();
            let input_block_id = block_id.clone();
            let input_format = output_format.clone();
            let mut input_render_version = render_version;
            rsx! {
                div {
                    class: "dxeditor__block",
                    "data-block-id": "{block_id_attr}",
                    "data-component": COMPONENT_PARAGRAPH,
                    "data-text-len": "{text_len}",
                    contenteditable,
                    oninput: move |event: FormEvent| {
                        apply_block_text_edit(
                            &input_catalog,
                            &input_state,
                            input_block_id.clone(),
                            event.value(),
                            &input_format,
                            on_change,
                            &mut input_render_version,
                        );
                    },
                    onkeydown: block_keydown_handler(
                        catalog,
                        editor_state,
                        block_id,
                        text_len,
                        output_format,
                        on_change,
                        render_version,
                        readonly,
                    ),
                    for segment in inline {
                        InlineNodeView {
                            node: segment.node,
                            start: segment.start,
                            end: segment.end,
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, PartialEq)]
struct InlineSegment {
    node: InlineNode,
    start: usize,
    end: usize,
}

#[component]
fn InlineNodeView(node: InlineNode, start: usize, end: usize) -> Element {
    let inline_id = node.id.0.clone();
    let text_len = node.text.chars().count();

    if node.component == COMPONENT_MENTION {
        let entity_id = node
            .attrs
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return rsx! {
            span {
                class: "dxeditor__mention",
                "data-inline-id": "{inline_id}",
                "data-inline-start": "{start}",
                "data-inline-end": "{end}",
                "data-inline-text-len": "{text_len}",
                "data-inline-prefix-len": "1",
                "data-entity-id": "{entity_id}",
                "@{node.text}"
            }
        };
    }

    rsx! {
        span {
            "data-inline-id": "{inline_id}",
            "data-inline-start": "{start}",
            "data-inline-end": "{end}",
            "data-inline-text-len": "{text_len}",
            "data-inline-prefix-len": "0",
            MarkedText {
                text: node.text,
                marks: node.marks,
                index: 0,
            }
        }
    }
}

#[component]
fn MarkedText(text: String, marks: Vec<Mark>, index: usize) -> Element {
    let Some(mark) = marks.get(index).cloned() else {
        return rsx! { span { "{text}" } };
    };
    let next = index + 1;
    match mark.component.as_str() {
        "bold" => rsx! {
            strong {
                MarkedText { text, marks, index: next }
            }
        },
        "italic" => rsx! {
            em {
                MarkedText { text, marks, index: next }
            }
        },
        "code" => rsx! {
            code {
                class: "dxeditor__inline-code",
                MarkedText { text, marks, index: next }
            }
        },
        COMPONENT_LINK => {
            let href = mark
                .attrs
                .get("href")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            rsx! {
                a {
                    href,
                    MarkedText { text, marks, index: next }
                }
            }
        }
        _ => rsx! {
            MarkedText { text, marks, index: next }
        },
    }
}

fn block_inline_segments(block: &BlockNode) -> Vec<InlineSegment> {
    match &block.content {
        crate::document::NodeContent::Inline(inline) => {
            let mut cursor = 0usize;
            inline
                .iter()
                .cloned()
                .map(|node| {
                    let start = cursor;
                    cursor += node.text.chars().count();
                    InlineSegment {
                        node,
                        start,
                        end: cursor,
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn block_keydown_handler(
    catalog: EditorCatalog,
    editor_state: EditorState,
    block_id: NodeId,
    text_len: usize,
    output_format: String,
    on_change: EventHandler<EditorPayload>,
    mut render_version: Signal<u64>,
    readonly: bool,
) -> impl FnMut(KeyboardEvent) {
    move |event: KeyboardEvent| {
        if readonly {
            return;
        }
        let key = event.key().to_string();
        let modifiers = event.modifiers();
        match key.as_str() {
            "Enter" => {
                event.prevent_default();
                spawn_split_block_command(
                    catalog.clone(),
                    editor_state.clone(),
                    block_id.clone(),
                    text_len,
                    output_format.clone(),
                    on_change,
                    render_version,
                );
            }
            "Backspace" => {
                let Some((command, args)) = editor_state
                    .selection()
                    .filter(EditorSelection::is_collapsed)
                    .filter(|selection| selection.focus.offset == 0)
                    .and_then(|selection| {
                        let document = editor_state.document();
                        previous_block_id(&document, &selection.focus.block_id).map(|previous_id| {
                            event.prevent_default();
                            (
                                "editor.merge_blocks",
                                json!({
                                "first_id": previous_id.0,
                                "second_id": selection.focus.block_id.0,
                                }),
                            )
                        })
                    })
                else {
                    return;
                };
                apply_editor_command(
                    &catalog,
                    &editor_state,
                    command,
                    args,
                    &output_format,
                    on_change,
                    &mut render_version,
                );
            }
            "b" | "B" if modifiers.ctrl() || modifiers.meta() => {
                event.prevent_default();
                spawn_toggle_mark_command(
                    catalog.clone(),
                    editor_state.clone(),
                    "bold",
                    output_format.clone(),
                    on_change,
                    render_version,
                );
            }
            "i" | "I" if modifiers.ctrl() || modifiers.meta() => {
                event.prevent_default();
                spawn_toggle_mark_command(
                    catalog.clone(),
                    editor_state.clone(),
                    "italic",
                    output_format.clone(),
                    on_change,
                    render_version,
                );
            }
            "e" | "E" if modifiers.ctrl() || modifiers.meta() => {
                event.prevent_default();
                spawn_toggle_mark_command(
                    catalog.clone(),
                    editor_state.clone(),
                    "code",
                    output_format.clone(),
                    on_change,
                    render_version,
                );
            }
            _ => {}
        }
    }
}

fn spawn_block_type_command(
    catalog: EditorCatalog,
    editor_state: EditorState,
    component: &'static str,
    attrs: Option<Value>,
    output_format: String,
    on_change: EventHandler<EditorPayload>,
    mut render_version: Signal<u64>,
) {
    spawn(async move {
        let Some(selection) = current_editor_selection(&editor_state).await else {
            return;
        };
        apply_editor_command(
            &catalog,
            &editor_state,
            "editor.set_block_type",
            block_type_args(&selection, component, attrs),
            &output_format,
            on_change,
            &mut render_version,
        );
    });
}

fn spawn_toggle_mark_command(
    catalog: EditorCatalog,
    editor_state: EditorState,
    mark: &'static str,
    output_format: String,
    on_change: EventHandler<EditorPayload>,
    mut render_version: Signal<u64>,
) {
    spawn(async move {
        let Some(selection) = current_editor_selection(&editor_state).await else {
            return;
        };
        if selection.is_collapsed() {
            return;
        }
        apply_editor_command(
            &catalog,
            &editor_state,
            "editor.toggle_mark",
            mark_args(&selection, mark),
            &output_format,
            on_change,
            &mut render_version,
        );
    });
}

fn spawn_split_block_command(
    catalog: EditorCatalog,
    editor_state: EditorState,
    block_id: NodeId,
    text_len: usize,
    output_format: String,
    on_change: EventHandler<EditorPayload>,
    mut render_version: Signal<u64>,
) {
    spawn(async move {
        let selection = current_editor_selection(&editor_state).await;
        let (id, offset) = selection
            .filter(EditorSelection::is_collapsed)
            .map(|selection| (selection.focus.block_id, selection.focus.offset))
            .unwrap_or((block_id, text_len));
        apply_editor_command(
            &catalog,
            &editor_state,
            "editor.split_block",
            json!({ "id": id.0, "offset": offset }),
            &output_format,
            on_change,
            &mut render_version,
        );
    });
}

async fn current_editor_selection(editor_state: &EditorState) -> Option<EditorSelection> {
    selection_bridge::browser_selection()
        .await
        .or_else(|| editor_state.selection())
}

fn block_type_args(selection: &EditorSelection, component: &str, attrs: Option<Value>) -> Value {
    let mut args = serde_json::Map::new();
    args.insert(
        "component".to_string(),
        Value::String(component.to_string()),
    );
    if let Some(attrs) = attrs {
        args.insert("attrs".to_string(), attrs);
    }
    args.insert(
        "id".to_string(),
        Value::String(selection.focus.block_id.0.clone()),
    );
    Value::Object(args)
}

fn mark_args(selection: &EditorSelection, mark: &str) -> Value {
    let mut args = serde_json::Map::new();
    args.insert("mark".to_string(), Value::String(mark.to_string()));
    args.insert("selection".to_string(), json!(selection));
    Value::Object(args)
}

fn previous_block_id(document: &EditorDocument, block_id: &NodeId) -> Option<NodeId> {
    let index = document
        .blocks
        .iter()
        .position(|block| block.id == *block_id)?;
    index
        .checked_sub(1)
        .and_then(|previous| document.blocks.get(previous))
        .map(|block| block.id.clone())
}

fn apply_block_text_edit(
    catalog: &EditorCatalog,
    editor_state: &EditorState,
    block_id: NodeId,
    text: String,
    output_format: &str,
    on_change: EventHandler<EditorPayload>,
    render_version: &mut Signal<u64>,
) {
    if editor_state
        .document()
        .blocks
        .iter()
        .find(|block| block.id == block_id)
        .is_some_and(|block| block.text_content() == text)
    {
        return;
    }

    apply_editor_command(
        catalog,
        editor_state,
        "editor.set_block_text",
        json!({ "id": block_id.0, "text": text }),
        output_format,
        on_change,
        render_version,
    );
}

fn apply_editor_command(
    catalog: &EditorCatalog,
    editor_state: &EditorState,
    command: &str,
    args: Value,
    output_format: &str,
    on_change: EventHandler<EditorPayload>,
    render_version: &mut Signal<u64>,
) {
    if dispatch_editor_command(
        catalog,
        editor_state,
        command,
        args,
        output_format,
        on_change,
    )
    .is_ok()
    {
        render_version.set(render_version() + 1);
    }
}

fn dispatch_editor_command(
    catalog: &EditorCatalog,
    editor_state: &EditorState,
    command: &str,
    args: Value,
    output_format: &str,
    on_change: EventHandler<EditorPayload>,
) -> Result<(), EditorError> {
    let transaction = catalog.commands().dispatch(command, editor_state, args)?;
    editor_state.apply_transaction(transaction)?;
    let payload = catalog
        .codecs()
        .encode(&editor_state.document(), output_format)?;
    on_change.call(payload);
    Ok(())
}

fn document_word_count(document: &EditorDocument) -> usize {
    document
        .text_content()
        .split_whitespace()
        .filter(|word| !word.is_empty())
        .count()
}

#[component]
pub fn MarkdownEditor(
    value: String,
    on_change: EventHandler<String>,
    #[props(default)] catalog: EditorCatalog,
    #[props(default)] readonly: bool,
) -> Element {
    rsx! {
        Editor {
            value: EditorPayload::new("markdown", Value::String(value)),
            output_format: "markdown".to_string(),
            catalog,
            readonly,
            on_change: move |payload: EditorPayload| {
                if let Some(value) = payload.value.as_str() {
                    on_change.call(value.to_string());
                }
            },
        }
    }
}

#[component]
pub fn PlainTextEditor(
    value: String,
    on_change: EventHandler<String>,
    #[props(default)] catalog: EditorCatalog,
    #[props(default)] readonly: bool,
) -> Element {
    rsx! {
        Editor {
            value: EditorPayload::new("plain_text", Value::String(value)),
            output_format: "plain_text".to_string(),
            catalog,
            readonly,
            on_change: move |payload: EditorPayload| {
                if let Some(value) = payload.value.as_str() {
                    on_change.call(value.to_string());
                }
            },
        }
    }
}

#[component]
pub fn DocumentEditor(
    document: EditorDocument,
    on_change: EventHandler<EditorDocument>,
    #[props(default)] catalog: EditorCatalog,
    #[props(default)] readonly: bool,
) -> Element {
    let value = serde_json::to_value(document).unwrap_or(Value::Null);
    rsx! {
        Editor {
            value: EditorPayload::new(DOCUMENT_SCHEMA_V1, value),
            output_format: DOCUMENT_SCHEMA_V1.to_string(),
            catalog,
            readonly,
            on_change: move |payload: EditorPayload| {
                if let Ok(document) = serde_json::from_value::<EditorDocument>(payload.value) {
                    on_change.call(document);
                }
            },
        }
    }
}

#[component]
pub fn DocumentView(document: EditorDocument, #[props(default)] catalog: EditorCatalog) -> Element {
    rsx! {
        DocumentEditor {
            document,
            catalog,
            readonly: true,
            on_change: move |_document: EditorDocument| {},
        }
    }
}
