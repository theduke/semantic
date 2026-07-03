use std::collections::BTreeMap;

use dioxus::prelude::*;
use serde_json::Value;

use crate::{
    catalog::EditorCatalog,
    codec::EditorPayload,
    document::{DOCUMENT_SCHEMA_V1, EditorDocument},
    state::EditorState,
};

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
    let text_value = state.read().document().text_content();

    let onchange_catalog = catalog.clone();
    let onchange_output_format = output_format.clone();
    let onchange_state = state.read().clone();

    rsx! {
        div { class: "dxeditor", "data-readonly": "{readonly}",
            textarea {
                class: "dxeditor__surface",
                value: "{text_value}",
                readonly,
                oninput: move |event: FormEvent| {
                    if readonly {
                        return;
                    }
                    if onchange_catalog
                        .commands()
                        .dispatch("editor.set_plain_text", &onchange_state, Value::String(event.value()))
                        .and_then(|transaction| onchange_state.apply_transaction(transaction))
                        .is_ok()
                    {
                        if let Ok(payload) = onchange_catalog
                            .codecs()
                            .encode(&onchange_state.document(), &onchange_output_format)
                        {
                            on_change.call(payload);
                        }
                    }
                },
            }
        }
    }
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
