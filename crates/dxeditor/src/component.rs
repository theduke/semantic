use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
};

use dioxus::prelude::*;
use serde_json::Value;

use crate::{
    bridge::{EditorBridge, run_engine_command},
    catalog::EditorCatalog,
    codec::EditorPayload,
    component_spec::validate_component_document,
    document::{
        BlockNode, COMPONENT_CODE, COMPONENT_DIVIDER, COMPONENT_HEADING, COMPONENT_LIST,
        COMPONENT_LIST_ITEM, COMPONENT_MENTION, COMPONENT_PARAGRAPH, COMPONENT_QUOTE,
        COMPONENT_TABLE, DOCUMENT_SCHEMA_V1, EditorDocument, InlineNode, Mark, NodeContent,
    },
    document_v2::{ComponentDocumentV2, UnknownComponentPolicy, ValidationLimits},
    format::{DecodeOptions, EncodeOptions},
    migrate::{migrate_v1_to_v2, migrate_v2_to_v1},
    protocol::{
        EngineCommand, EngineEvent, HistoryPolicy, MentionSuggestion, ProtocolSession, Revision,
    },
    suggestion::SuggestionQueryContext,
};

static NEXT_EDITOR_ID: AtomicU64 = AtomicU64::new(1);

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

.dxeditor[data-engine="tiptap-prosemirror"] {
  position: relative;
  overflow: visible;
  border: 0;
  border-radius: 0;
  box-shadow: none;
}

.dxeditor__editor-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: 38px;
  padding: 4px 8px;
  color: #617084;
}

.dxeditor__editor-actions-status {
  margin-left: auto;
  font-size: 12px;
}

.dxeditor__live-status {
  position: absolute;
  width: 1px;
  height: 1px;
  padding: 0;
  margin: -1px;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
}

.dxeditor[data-engine="tiptap-prosemirror"] .dxeditor__document {
  display: block;
  min-height: 260px;
  padding: 22px 48px;
}

.dxeditor-engine__content {
  min-height: 216px;
  outline: none;
  line-height: 1.62;
  overflow-wrap: anywhere;
}

.dxeditor-engine__content > * {
  margin: 0.2rem 0;
}

.dxeditor-engine__content ul[data-type="taskList"],
.dxeditor__task-list {
  margin-left: 0;
  padding-left: 0;
  list-style: none;
}

.dxeditor-engine__content ul[data-type="taskList"] > li,
.dxeditor__task-item {
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
}

.dxeditor-engine__content ul[data-type="taskList"] > li > label,
.dxeditor__task-item > input[type="checkbox"] {
  flex: 0 0 auto;
  margin-top: 0.35em;
}

.dxeditor-engine__content ul[data-type="taskList"] > li > label > input {
  margin: 0;
}

.dxeditor-engine__content ul[data-type="taskList"] > li > div,
.dxeditor__task-item-content {
  flex: 1 1 auto;
  min-width: 0;
}

.dxeditor-engine__content ul[data-type="taskList"] > li > div > :first-child,
.dxeditor__task-item-content > :first-child {
  margin-top: 0;
}

.dxeditor-engine__content ul[data-type="taskList"] > li > div > :last-child,
.dxeditor__task-item-content > :last-child {
  margin-bottom: 0;
}

.dxeditor-engine__content h1,
.dxeditor-engine__content h2,
.dxeditor-engine__content h3 {
  line-height: 1.25;
  margin-top: 1.1em;
}

.dxeditor-engine__content blockquote {
  margin-left: 0;
  border-left: 3px solid #9bb5c1;
  padding-left: 12px;
  color: #435367;
}

.dxeditor-engine__content pre {
  overflow-x: auto;
  border-radius: 6px;
  background: #f6f8fa;
  padding: 12px;
}

.dxeditor-engine__content table {
  width: 100%;
  table-layout: fixed;
  border-collapse: collapse;
  overflow: hidden;
}

.dxeditor-engine__content td,
.dxeditor-engine__content th {
  min-width: 96px;
  border: 1px solid #cfd8e3;
  padding: 6px 8px;
  vertical-align: top;
}

.dxeditor-engine__content .selectedCell::after {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background: rgb(23 107 135 / 14%);
  content: "";
}

.dxeditor-engine__content .tableWrapper {
  overflow-x: auto;
}

.dxeditor-engine__mention {
  border-radius: 4px;
  background: #e9f2f5;
  color: #176b87;
  padding: 1px 4px;
}

.dxeditor-engine__opaque {
  border: 1px dashed #a8b4c2;
  white-space: pre-wrap;
}

.dxeditor__overlays {
  position: absolute;
  inset: 0;
  z-index: 20;
  pointer-events: none;
}

.dxeditor-engine__surface {
  position: absolute;
  display: flex;
  gap: 2px;
  max-width: min(520px, calc(100vw - 24px));
  border: 1px solid #d8dee7;
  border-radius: 7px;
  background: #fff;
  box-shadow: 0 6px 24px rgb(23 32 42 / 16%);
  padding: 4px;
  pointer-events: auto;
}

.dxeditor-engine__surface[hidden] {
  display: none;
}

.dxeditor-engine__button {
  min-width: 30px;
  min-height: 30px;
  border: 0;
  border-radius: 5px;
  background: transparent;
  color: inherit;
  padding: 4px 8px;
  font: inherit;
  cursor: pointer;
}

.dxeditor-engine__button:hover,
.dxeditor-engine__button:focus-visible {
  background: #eef2f6;
  outline: 2px solid transparent;
}

.dxeditor-engine__slash {
  flex-direction: column;
  width: 230px;
  max-height: 300px;
  overflow-y: auto;
}

.dxeditor-engine__block-menu {
  flex-direction: column;
  min-width: 160px;
}

.dxeditor-engine__block-menu .dxeditor-engine__button {
  text-align: left;
}

.dxeditor-engine__link-popover {
  flex-direction: column;
  width: min(320px, calc(100vw - 24px));
  padding: 8px;
}

.dxeditor-engine__link-popover label {
  display: grid;
  gap: 4px;
  color: #435367;
  font-size: 12px;
}

.dxeditor-engine__link-popover input {
  min-height: 34px;
  border: 1px solid #a8b4c2;
  border-radius: 5px;
  padding: 4px 8px;
  font: inherit;
}

.dxeditor-engine__link-actions {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
}

.dxeditor-engine__field-error {
  min-height: 1em;
  color: #b42318;
  font-size: 12px;
}

.dxeditor-engine__slash .dxeditor-engine__button {
  text-align: left;
}

.dxeditor-engine__block-controls {
  transform: translateX(-100%);
}

.dxeditor__error {
  border: 1px solid #b42318;
  border-radius: 6px;
  background: #fff5f4;
  padding: 12px;
  color: #7a271a;
}

@media (max-width: 640px) {
  .dxeditor[data-engine="tiptap-prosemirror"] .dxeditor__document {
    padding: 18px 20px;
  }

  .dxeditor-engine__table-controls {
    position: fixed;
    right: 8px;
    bottom: max(8px, env(safe-area-inset-bottom));
    left: 8px !important;
    top: auto !important;
    overflow-x: auto;
  }
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

/// Controls the optional document-wide action strip. Formatting and insertion
/// actions are intentionally never shown in this surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorActionsMode {
    #[default]
    Hidden,
    History,
    HistoryAndStatus,
}

#[component]
pub fn Editor(
    value: EditorPayload,
    output_format: String,
    catalog: EditorCatalog,
    on_change: EventHandler<EditorPayload>,
    #[props(default)] onfocus: Option<EventHandler<FocusEvent>>,
    #[props(default)] onblur: Option<EventHandler<FocusEvent>>,
    #[props(default)] readonly: bool,
    #[props(default)] editor_actions: EditorActionsMode,
    #[props(default)] external_revision: Option<u64>,
    #[props(default = "Document editor".to_string())] aria_label: String,
) -> Element {
    let decoded = decode_editor_payload(&catalog, &value);
    let decode_error = decoded.as_ref().err().map(ToString::to_string);
    let initial_document = decoded.unwrap_or_default();
    let editor_id = use_signal(|| {
        format!(
            "dxeditor-{}",
            NEXT_EDITOR_ID.fetch_add(1, Ordering::Relaxed)
        )
    })();
    let session_id = use_signal(|| {
        format!(
            "dxeditor-session-{}",
            NEXT_EDITOR_ID.fetch_add(1, Ordering::Relaxed)
        )
    })();
    let schema_fingerprint = catalog.schema_fingerprint();
    let engine_manifest = crate::engine_manifest::EditorEngineManifest::new(
        catalog.component_specs(),
        output_format.clone(),
        aria_label.clone(),
    );
    let initial_external_revision = Revision(external_revision.unwrap_or_default());
    let mut status = use_signal(|| "Ready".to_string());
    let mut block_count = use_signal(|| initial_document.root.content.len());
    let mut word_count = use_signal(|| document_word_count(&initial_document));
    let mut current_document = use_signal(|| initial_document.clone());
    let initial_external = (value.clone(), external_revision);
    let mut observed_external = use_signal(move || initial_external);
    let mut last_emitted = use_signal(|| None::<(EditorPayload, Revision)>);
    let mut dirty = use_signal(|| false);
    let mut current_external_revision = use_signal(|| initial_external_revision);
    let mut last_local_revision = use_signal(|| Revision::ZERO);
    let mut can_undo = use_signal(|| false);
    let mut can_redo = use_signal(|| false);
    let output_catalog = catalog.clone();
    let output_format_for_event = output_format.clone();
    let expected_session_id = session_id.clone();
    let expected_schema_fingerprint = schema_fingerprint.clone();
    let undo_session = session_id.clone();
    let undo_schema_fingerprint = schema_fingerprint.clone();
    let redo_session = session_id.clone();
    let redo_schema_fingerprint = schema_fingerprint.clone();
    let external_catalog = catalog.clone();
    let external_session = session_id.clone();
    let external_schema_fingerprint = schema_fingerprint.clone();
    let mention_catalog = catalog.clone();
    let mention_session_id = session_id.clone();
    let mention_schema_fingerprint = schema_fingerprint.clone();
    let external_value = value.clone();
    let supplied_external_revision = external_revision;
    use_effect(use_reactive!(|(
        external_value,
        supplied_external_revision,
    )| {
        let observed = (external_value.clone(), supplied_external_revision);
        if *observed_external.peek() == observed {
            return;
        }
        observed_external.set(observed);

        let current_revision = current_external_revision();
        let supplied_revision = supplied_external_revision.map(Revision);
        if supplied_revision.is_some_and(|revision| revision < current_revision) {
            status.set(format!(
                "Stale external revision {} ignored",
                supplied_revision.unwrap_or_default()
            ));
            return;
        }

        if last_emitted
            .peek()
            .as_ref()
            .is_some_and(|(payload, _)| payload == &external_value)
        {
            if let Some(revision) =
                supplied_revision.filter(|revision| *revision > current_revision)
            {
                let session = ProtocolSession::new(
                    external_session.clone(),
                    external_schema_fingerprint.clone(),
                    revision,
                );
                run_engine_command(&EngineCommand::Acknowledge {
                    session,
                    local_revision: last_local_revision(),
                });
                current_external_revision.set(revision);
            }
            last_emitted.set(None);
            dirty.set(false);
            status.set("Saved".to_string());
            return;
        }

        let incoming_revision = match supplied_revision {
            Some(revision) if revision <= current_revision => {
                status.set(format!("External revision {revision} ignored"));
                return;
            }
            Some(revision) => revision,
            None => current_revision.next(),
        };
        if dirty() {
            status.set(format!(
                "Conflict: external revision {incoming_revision} was not applied"
            ));
            return;
        }
        match decode_editor_payload(&external_catalog, &external_value) {
            Ok(document) => {
                block_count.set(document.root.content.len());
                word_count.set(document_word_count(&document));
                current_document.set(document.clone());
                let session = ProtocolSession::new(
                    external_session.clone(),
                    external_schema_fingerprint.clone(),
                    incoming_revision,
                );
                run_engine_command(&EngineCommand::ReplaceDocument {
                    session,
                    document,
                    history_policy: HistoryPolicy::Reset,
                });
                current_external_revision.set(incoming_revision);
                status.set(format!("Updated to external revision {incoming_revision}"));
            }
            Err(error) => status.set(format!("External update rejected: {error}")),
        }
    }));

    rsx! {
        style { {DXEDITOR_STYLE} }
        div {
            class: "dxeditor",
            "data-readonly": "{readonly}",
            "data-engine": "tiptap-prosemirror",
            onfocus: move |event| if let Some(handler) = onfocus { handler.call(event) },
            onblur: move |event| if let Some(handler) = onblur { handler.call(event) },
            if !readonly && editor_actions != EditorActionsMode::Hidden {
                div {
                    class: "dxeditor__editor-actions",
                    role: "toolbar",
                    aria_label: "Editor history",
                    button {
                        class: "dxeditor__button",
                        r#type: "button",
                        aria_label: "Undo",
                        disabled: !can_undo(),
                            onclick: move |_| run_history_command(
                                &undo_session,
                                &undo_schema_fingerprint,
                                current_external_revision(),
                                last_local_revision(),
                                "undo",
                            ),
                        "↶"
                    }
                    button {
                        class: "dxeditor__button",
                        r#type: "button",
                        aria_label: "Redo",
                        disabled: !can_redo(),
                            onclick: move |_| run_history_command(
                                &redo_session,
                                &redo_schema_fingerprint,
                                current_external_revision(),
                                last_local_revision(),
                                "redo",
                            ),
                        "↷"
                    }
                    if editor_actions == EditorActionsMode::HistoryAndStatus {
                        span {
                            class: "dxeditor__editor-actions-status",
                            "{status} · {block_count} blocks · {word_count} words"
                        }
                    }
                }
            }
            if let Some(error) = decode_error {
                div {
                    class: "dxeditor__error",
                    role: "alert",
                    p { "This document could not be opened safely: {error}" }
                    details {
                        summary { "View original source" }
                        pre { "{value.value}" }
                    }
                }
            } else if readonly {
                crate::render_v2::ReadOnlyDocumentV2 { document: initial_document }
            } else {
                div {
                    class: "dxeditor__document",
                    "data-dxeditor-host": "{editor_id}",
                    aria_label: "{aria_label}",
                }
                div { class: "dxeditor__overlays", "data-dxeditor-overlays": "true" }
                EditorBridge {
                    editor_id: editor_id.clone(),
                    session: ProtocolSession::new(
                        session_id.clone(),
                        schema_fingerprint.clone(),
                        initial_external_revision,
                    ),
                    document_value: initial_document,
                    readonly,
                    aria_label: aria_label.clone(),
                    format_id: output_format.clone(),
                    manifest: engine_manifest,
                    on_event: move |event| {
                        match event {
                            EngineEvent::Ready { session }
                                if session_matches(
                                    &session,
                                    &expected_session_id,
                                    &expected_schema_fingerprint,
                                    current_external_revision(),
                                ) =>
                            {
                                status.set("Ready".to_string());
                            }
                            EngineEvent::DocumentChange { session, revision, document }
                                if session_matches(
                                    &session,
                                    &expected_session_id,
                                    &expected_schema_fingerprint,
                                    current_external_revision(),
                                ) && revision > last_local_revision() =>
                            {
                                if let Err(error) = validate_browser_snapshot(&output_catalog, &document) {
                                    status.set(format!("Editor snapshot rejected: {error}"));
                                    dirty.set(true);
                                    return;
                                }
                                last_local_revision.set(revision);
                                block_count.set(document.root.content.len());
                                word_count.set(document_word_count(&document));
                                current_document.set(document.clone());
                                dirty.set(true);
                                match encode_editor_payload(&output_catalog, &document, &output_format_for_event) {
                                    Ok(payload) => {
                                        status.set(format!("Unsaved revision {revision}"));
                                        last_emitted.set(Some((payload.clone(), revision)));
                                        on_change.call(payload);
                                    }
                                    Err(error) => status.set(format!("Encoding failed: {error}")),
                                }
                            }
                            EngineEvent::Blur { session, revision, document }
                                if session_matches(
                                    &session,
                                    &expected_session_id,
                                    &expected_schema_fingerprint,
                                    current_external_revision(),
                                ) && revision >= last_local_revision() =>
                            {
                                if let Err(error) = validate_browser_snapshot(&output_catalog, &document) {
                                    status.set(format!("Editor snapshot rejected on blur: {error}"));
                                    dirty.set(true);
                                    return;
                                }
                                // Document changes are emitted synchronously. Blur is only a
                                // durability/focus boundary and must not duplicate `on_change`.
                                last_local_revision.set(revision);
                                status.set(if dirty() { format!("Unsaved revision {revision}") } else { "Saved".to_string() });
                            }
                            EngineEvent::MentionQuery { session, request_id, query }
                                if session_matches(
                                    &session,
                                    &expected_session_id,
                                    &expected_schema_fingerprint,
                                    current_external_revision(),
                                ) =>
                            {
                                let Some(provider) = mention_catalog.suggestions().provider('@') else {
                                    return;
                                };
                                let catalog = mention_catalog.clone();
                                let document = current_document();
                                let response_session = ProtocolSession::new(
                                    mention_session_id.clone(),
                                    mention_schema_fingerprint.clone(),
                                    current_external_revision(),
                                );
                                spawn(async move {
                                    let legacy = migrate_v2_to_v1(&document)
                                        .unwrap_or_else(|_| EditorDocument::plain_text(document.text_content()));
                                    let items = provider
                                        .query(SuggestionQueryContext {
                                            query,
                                            state: crate::state::EditorState::new(legacy),
                                            catalog,
                                        })
                                        .await;
                                    let suggestions = items
                                        .into_iter()
                                        .map(|item| MentionSuggestion {
                                            id: item.id,
                                            label: item.label,
                                            detail: item.description,
                                        })
                                        .collect();
                                    run_engine_command(&EngineCommand::MentionSuggestions {
                                        session: response_session,
                                        request_id,
                                        suggestions,
                                    });
                                });
                            }
                            EngineEvent::CommandState { session, can_undo: next_can_undo, can_redo: next_can_redo }
                                if session_matches(
                                    &session,
                                    &expected_session_id,
                                    &expected_schema_fingerprint,
                                    current_external_revision(),
                                ) =>
                            {
                                can_undo.set(next_can_undo);
                                can_redo.set(next_can_redo);
                            }
                            EngineEvent::Error { session, message, .. }
                                if session_matches(
                                    &session,
                                    &expected_session_id,
                                    &expected_schema_fingerprint,
                                    current_external_revision(),
                                ) =>
                            {
                                status.set(format!("Editor error: {message}"));
                            }
                            _ => {}
                        }
                    },
                }
            }
            span { class: "dxeditor__live-status", role: "status", "{status}" }
        }
    }
}

fn decode_editor_payload(
    catalog: &EditorCatalog,
    payload: &EditorPayload,
) -> Result<ComponentDocumentV2, String> {
    if catalog.document_formats().format(&payload.format).is_some() {
        return catalog
            .document_formats()
            .decode(payload, catalog.component_specs(), DecodeOptions::default())
            .map(|decoded| decoded.document)
            .map_err(|error| error.to_string());
    }

    let legacy = catalog
        .codecs()
        .decode(payload)
        .map_err(|error| error.to_string())?;
    let document = migrate_v1_to_v2(&legacy).map_err(|error| error.to_string())?;
    validate_component_document(
        &document,
        catalog.component_specs(),
        &ValidationLimits::default(),
        UnknownComponentPolicy::PreserveOpaque,
    )
    .map_err(|error| error.to_string())?;
    Ok(document)
}

fn encode_editor_payload(
    catalog: &EditorCatalog,
    document: &ComponentDocumentV2,
    format: &str,
) -> Result<EditorPayload, String> {
    if catalog.document_formats().format(format).is_some() {
        return catalog
            .document_formats()
            .encode(
                document,
                format,
                catalog.component_specs(),
                EncodeOptions::default(),
            )
            .map(|encoded| encoded.payload)
            .map_err(|error| error.to_string());
    }

    let legacy = migrate_v2_to_v1(document).map_err(|error| error.to_string())?;
    catalog
        .codecs()
        .encode(&legacy, format)
        .map_err(|error| error.to_string())
}

fn validate_browser_snapshot(
    catalog: &EditorCatalog,
    document: &ComponentDocumentV2,
) -> Result<(), String> {
    validate_component_document(
        document,
        catalog.component_specs(),
        &ValidationLimits::default(),
        UnknownComponentPolicy::PreserveOpaque,
    )
    .map_err(|error| {
        error
            .issues
            .first()
            .map(|issue| format!("{} at {}: {}", issue.code, issue.path, issue.message))
            .unwrap_or_else(|| error.to_string())
    })
}

fn session_matches(
    session: &ProtocolSession,
    session_id: &str,
    schema_fingerprint: &str,
    external_revision: Revision,
) -> bool {
    session.protocol_version == crate::protocol::EDITOR_PROTOCOL_VERSION
        && session.session_id == session_id
        && session.schema_fingerprint == schema_fingerprint
        && session.external_revision == external_revision
}

fn run_history_command(
    session_id: &str,
    schema_fingerprint: &str,
    external_revision: Revision,
    expected_revision: Revision,
    command: &str,
) {
    run_engine_command(&EngineCommand::RunCommand {
        session: ProtocolSession::new(session_id, schema_fingerprint, external_revision),
        request_id: NEXT_EDITOR_ID.fetch_add(1, Ordering::Relaxed),
        expected_revision,
        command: command.to_string(),
        args: Value::Null,
    });
}

#[component]
fn ReadOnlyDocument(document: EditorDocument) -> Element {
    rsx! {
        article { class: "dxeditor__document dxeditor__read-only",
            for block in document.blocks {
                ReadOnlyBlock { block }
            }
        }
    }
}

#[component]
fn ReadOnlyBlock(block: BlockNode) -> Element {
    let component = block.component.as_str();
    let level = block
        .attrs
        .get("level")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 6);
    match component {
        COMPONENT_HEADING => match level {
            1 => rsx! { h1 { ReadOnlyContent { content: block.content } } },
            2 => rsx! { h2 { ReadOnlyContent { content: block.content } } },
            3 => rsx! { h3 { ReadOnlyContent { content: block.content } } },
            4 => rsx! { h4 { ReadOnlyContent { content: block.content } } },
            5 => rsx! { h5 { ReadOnlyContent { content: block.content } } },
            _ => rsx! { h6 { ReadOnlyContent { content: block.content } } },
        },
        COMPONENT_QUOTE => rsx! { blockquote { ReadOnlyContent { content: block.content } } },
        COMPONENT_CODE => rsx! { pre { code { "{block.text_content()}" } } },
        COMPONENT_DIVIDER => rsx! { hr {} },
        COMPONENT_LIST => {
            let ordered = block
                .attrs
                .get("ordered")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let task = block
                .attrs
                .get("task")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let NodeContent::Blocks(items) = block.content else {
                return rsx! {};
            };
            if ordered {
                rsx! { ol { for item in items { ReadOnlyListItem { block: item } } } }
            } else {
                rsx! { ul {
                    class: if task { "dxeditor__task-list" } else { "" },
                    for item in items { ReadOnlyListItem { block: item } }
                } }
            }
        }
        COMPONENT_TABLE => {
            let NodeContent::Table(table) = block.content else {
                return rsx! {};
            };
            rsx! {
                div { class: "tableWrapper",
                    table {
                        tbody {
                            for row in table.rows {
                                tr {
                                    for cell in row.cells {
                                        td {
                                            for cell_block in cell.blocks {
                                                ReadOnlyBlock { block: cell_block }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        "opaque_markdown" | "opaque_markdown_block" => {
            let source = block
                .attrs
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or_default();
            rsx! { pre { class: "dxeditor-engine__opaque", code { "{source}" } } }
        }
        COMPONENT_PARAGRAPH | COMPONENT_LIST_ITEM => {
            rsx! { p { ReadOnlyContent { content: block.content } } }
        }
        _ => rsx! { div { class: "dxeditor__fallback", "{block.text_content()}" } },
    }
}

#[component]
fn ReadOnlyListItem(block: BlockNode) -> Element {
    let checked = block.attrs.get("checked").and_then(Value::as_bool);
    rsx! {
        li {
            class: if checked.is_some() { "dxeditor__task-item" } else { "" },
            if let Some(checked) = checked {
                input { r#type: "checkbox", checked, disabled: true, aria_label: "Task complete" }
                div { class: "dxeditor__task-item-content",
                    ReadOnlyContent { content: block.content }
                }
            } else {
                ReadOnlyContent { content: block.content }
            }
        }
    }
}

#[component]
fn ReadOnlyContent(content: NodeContent) -> Element {
    match content {
        NodeContent::Inline(inline) => rsx! {
            for node in inline { ReadOnlyInline { node } }
        },
        NodeContent::Blocks(blocks) => rsx! {
            for block in blocks { ReadOnlyBlock { block } }
        },
        NodeContent::Table(_) | NodeContent::Void | NodeContent::Custom(_) => rsx! {},
    }
}

#[component]
fn ReadOnlyInline(node: InlineNode) -> Element {
    if node.component == COMPONENT_MENTION {
        return rsx! { span { class: "dxeditor-engine__mention", "@{node.text}" } };
    }
    render_marked_text(node.text, &node.marks, 0)
}

fn render_marked_text(text: String, marks: &[Mark], index: usize) -> Element {
    let Some(mark) = marks.get(index) else {
        return rsx! { "{text}" };
    };
    let child = render_marked_text(text, marks, index + 1);
    match mark.component.as_str() {
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
            if safe_link_url(href) {
                rsx! { a { href, rel: "noopener noreferrer", {child} } }
            } else {
                child
            }
        }
        _ => child,
    }
}

fn safe_link_url(url: &str) -> bool {
    crate::component_spec::is_safe_url(url)
}

fn document_word_count(document: &ComponentDocumentV2) -> usize {
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
    #[props(default)] onfocus: Option<EventHandler<FocusEvent>>,
    #[props(default)] onblur: Option<EventHandler<FocusEvent>>,
    #[props(default)] catalog: EditorCatalog,
    #[props(default)] readonly: bool,
    #[props(default)] editor_actions: EditorActionsMode,
    #[props(default)] external_revision: Option<u64>,
) -> Element {
    rsx! {
        Editor {
            value: EditorPayload::new("markdown", Value::String(value)),
            output_format: "markdown".to_string(),
            catalog,
            readonly,
            editor_actions,
            external_revision,
            onfocus,
            onblur,
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
    #[props(default)] editor_actions: EditorActionsMode,
    #[props(default)] external_revision: Option<u64>,
) -> Element {
    rsx! {
        Editor {
            value: EditorPayload::new("plain_text", Value::String(value)),
            output_format: "plain_text".to_string(),
            catalog,
            readonly,
            editor_actions,
            external_revision,
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
    #[props(default)] editor_actions: EditorActionsMode,
    #[props(default)] external_revision: Option<u64>,
) -> Element {
    let value = serde_json::to_value(document).unwrap_or(Value::Null);
    rsx! {
        Editor {
            value: EditorPayload::new(DOCUMENT_SCHEMA_V1, value),
            output_format: DOCUMENT_SCHEMA_V1.to_string(),
            catalog,
            readonly,
            editor_actions,
            external_revision,
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
    let _ = catalog;
    rsx! { ReadOnlyDocument { document } }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_items_share_a_single_row_layout_in_editable_and_readonly_views() {
        assert!(
            DXEDITOR_STYLE.contains(".dxeditor-engine__content ul[data-type=\"taskList\"] > li,")
        );
        assert!(DXEDITOR_STYLE.contains(".dxeditor__task-item {\n  display: flex;"));
        assert!(DXEDITOR_STYLE.contains(".dxeditor__task-item-content {"));
    }

    #[test]
    fn legacy_v1_payloads_cross_the_component_boundary_as_v2() {
        let catalog = EditorCatalog::default();
        let payload = EditorPayload::new(
            DOCUMENT_SCHEMA_V1,
            serde_json::to_value(EditorDocument::plain_text("hello")).unwrap(),
        );
        let document = decode_editor_payload(&catalog, &payload).unwrap();
        assert_eq!(document.version, 2);
        assert_eq!(document.text_content(), "hello");

        let encoded = encode_editor_payload(&catalog, &document, DOCUMENT_SCHEMA_V1).unwrap();
        let legacy: EditorDocument = serde_json::from_value(encoded.value).unwrap();
        assert_eq!(legacy.text_content(), "hello");
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn markdown_compatibility_uses_the_v2_session_document() {
        let catalog = EditorCatalog::default();
        let payload = EditorPayload::new("markdown", Value::String("# Hello\n".to_string()));
        let document = decode_editor_payload(&catalog, &payload).unwrap();
        assert_eq!(document.version, 2);
        assert_eq!(document.text_content(), "Hello");

        let encoded = encode_editor_payload(&catalog, &document, "markdown").unwrap();
        assert!(encoded.value.as_str().unwrap().contains("# Hello"));
    }

    #[test]
    fn browser_snapshots_are_validated_before_component_state_acceptance() {
        let catalog = EditorCatalog::default();
        let mut duplicate = ComponentDocumentV2::plain_text("one");
        let cloned = duplicate.root.content[0].clone();
        duplicate.root.content.push(cloned);
        let error = validate_browser_snapshot(&catalog, &duplicate).unwrap_err();
        assert!(error.contains("duplicate_node_id"));

        let mut missing = ComponentDocumentV2::plain_text("one");
        missing.root.content[0].id = None;
        let error = validate_browser_snapshot(&catalog, &missing).unwrap_err();
        assert!(error.contains("missing_node_id"));
    }
}
