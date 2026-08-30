use dioxus::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{
    document_v2::ComponentDocumentV2,
    engine_manifest::EditorEngineManifest,
    protocol::{EngineCommand, EngineEvent, ProtocolSession, SessionRevisionGuard},
};

const ENGINE_SCRIPT: &str = include_str!("../../web/dist/editor.iife.js");
static ENGINE_INSTALL_STARTED: AtomicBool = AtomicBool::new(false);

#[component]
pub(crate) fn EditorBridge(
    editor_id: String,
    session: ProtocolSession,
    document_value: ComponentDocumentV2,
    readonly: bool,
    aria_label: String,
    format_id: String,
    manifest: EditorEngineManifest,
    entity_link_color: Option<String>,
    on_event: EventHandler<EngineEvent>,
) -> Element {
    let destroy = EngineCommand::Destroy {
        session: session.clone(),
    };
    use_drop(move || run_engine_command(&destroy));

    use_effect(move || {
        let mount = EngineCommand::Mount {
            session: session.clone(),
            document: document_value.clone(),
            readonly,
            aria_label: aria_label.clone(),
            format_id: format_id.clone(),
            manifest: manifest.clone(),
        };
        let Ok(editor_id_json) = serde_json::to_string(&editor_id) else {
            return;
        };
        let Ok(session_json) = serde_json::to_string(&session) else {
            return;
        };
        let Ok(document_json) = serde_json::to_string(&document_value) else {
            return;
        };
        let readonly = matches!(mount, EngineCommand::Mount { readonly: true, .. });
        let Ok(aria_label_json) = serde_json::to_string(&aria_label) else {
            return;
        };
        let Ok(format_id_json) = serde_json::to_string(&format_id) else {
            return;
        };
        let Ok(manifest_json) = serde_json::to_string(&manifest) else {
            return;
        };
        let Ok(entity_link_color_json) = serde_json::to_string(&entity_link_color) else {
            return;
        };
        let engine_bootstrap = if ENGINE_INSTALL_STARTED.swap(true, Ordering::AcqRel) {
            ""
        } else {
            ENGINE_SCRIPT
        };
        let script = format!(
            r#"
{engine_bootstrap}
const editorId = {editor_id_json};
const protocol = {session_json};
const sessionId = protocol.session_id;
window.__semanticDxEditorProtocol ??= new Map();
window.__semanticDxEditorMentionRequests ??= new Map();
window.__semanticDxEditorEntityRequests ??= new Map();
const protocolState = {{ ...protocol, lastLocalRevision: 0 }};
window.__semanticDxEditorProtocol.set(sessionId, protocolState);
const host = document.querySelector(`[data-dxeditor-host="${{CSS.escape(editorId)}}"]`);
if (!host || !window.__semanticDxEditor) {{
  dioxus.send({{ kind: 'error', ...protocolState, message: 'editor host or engine unavailable' }});
}} else {{
  try {{
    window.__semanticDxEditor.mount(sessionId, host, {{
      sessionId,
      schemaFingerprint: protocol.schema_fingerprint,
      readonly: {readonly},
      ariaLabel: {aria_label_json},
      formatId: {format_id_json},
      manifest: {manifest_json},
      entityLinkColor: {entity_link_color_json},
      document: {document_json},
      mentionProvider: (query, context) => new Promise((resolve, reject) => {{
        const requestId = (protocolState.nextMentionRequestId ?? 0) + 1;
        protocolState.nextMentionRequestId = requestId;
        const requests = window.__semanticDxEditorMentionRequests.get(sessionId) ?? new Map();
        window.__semanticDxEditorMentionRequests.set(sessionId, requests);
        requests.set(requestId, resolve);
        context.signal.addEventListener('abort', () => {{
          requests.delete(requestId);
          reject(new DOMException('Aborted', 'AbortError'));
        }}, {{ once: true }});
        dioxus.send({{ kind: 'mentionQuery', ...protocolState, request_id: requestId, query }});
      }}),
      entitySearchProvider: {entity_link_color_json} === null ? undefined : (query, context) => new Promise((resolve, reject) => {{
        const requestId = (protocolState.nextEntityRequestId ?? 0) + 1;
        protocolState.nextEntityRequestId = requestId;
        const requests = window.__semanticDxEditorEntityRequests.get(sessionId) ?? new Map();
        window.__semanticDxEditorEntityRequests.set(sessionId, requests);
        requests.set(requestId, resolve);
        context.signal.addEventListener('abort', () => {{
          requests.delete(requestId);
          reject(new DOMException('Aborted', 'AbortError'));
        }}, {{ once: true }});
        dioxus.send({{ kind: 'entitySearchQuery', ...protocolState, request_id: requestId, query }});
      }}),
      entityPreviewProvider: {entity_link_color_json} === null ? undefined : (entityId, context) => new Promise((resolve, reject) => {{
        const requestId = (protocolState.nextEntityRequestId ?? 0) + 1;
        protocolState.nextEntityRequestId = requestId;
        const requests = window.__semanticDxEditorEntityRequests.get(sessionId) ?? new Map();
        window.__semanticDxEditorEntityRequests.set(sessionId, requests);
        requests.set(requestId, resolve);
        context.signal.addEventListener('abort', () => {{
          requests.delete(requestId);
          reject(new DOMException('Aborted', 'AbortError'));
        }}, {{ once: true }});
        dioxus.send({{ kind: 'entityPreviewQuery', ...protocolState, request_id: requestId, entity_id: entityId }});
      }}),
      entityOpenHandler: {entity_link_color_json} === null ? undefined : entityId => {{
        dioxus.send({{ kind: 'entityOpen', ...protocolState, entity_id: entityId }});
      }},
      emit: event => {{
        const current = window.__semanticDxEditorProtocol.get(sessionId) ?? protocolState;
        if (Number.isSafeInteger(event.revision)) current.lastLocalRevision = event.revision;
        dioxus.send({{ ...event, ...current }});
      }},
    }});
  }} catch (error) {{
    dioxus.send({{ kind: 'error', ...protocolState, message: String(error) }});
  }}
}}
"#
        );
        let guard_session = session.clone();
        let mut eval = document::eval(&script);
        spawn(async move {
            let mut guard = SessionRevisionGuard::new(guard_session);
            while let Ok(event) = eval.recv::<EngineEvent>().await {
                if guard.accept(&event).is_ok() {
                    on_event.call(event);
                }
            }
        });
    });
    rsx! {}
}

pub(crate) fn run_engine_command(command: &EngineCommand) {
    let script = match command {
        EngineCommand::RunCommand {
            session,
            expected_revision,
            command,
            args,
            ..
        } => {
            let Ok(session_id) = serde_json::to_string(&session.session_id) else {
                return;
            };
            let Ok(schema_fingerprint) = serde_json::to_string(&session.schema_fingerprint) else {
                return;
            };
            let Ok(command) = serde_json::to_string(command) else {
                return;
            };
            format!(
                r#"(() => {{
const state = window.__semanticDxEditorProtocol?.get({session_id});
if (!state || state.protocol_version !== {} || state.schema_fingerprint !== {schema_fingerprint}
    || state.external_revision !== {} || state.lastLocalRevision !== {}) return;
window.__semanticDxEditor?.command({session_id}, {command}, {args});
}})();"#,
                session.protocol_version, session.external_revision.0, expected_revision.0
            )
        }
        EngineCommand::ReplaceDocument {
            session,
            document,
            history_policy,
        } => {
            let Ok(session_id) = serde_json::to_string(&session.session_id) else {
                return;
            };
            let Ok(schema_fingerprint) = serde_json::to_string(&session.schema_fingerprint) else {
                return;
            };
            let Ok(document) = serde_json::to_string(document) else {
                return;
            };
            let Ok(history_policy) = serde_json::to_string(history_policy) else {
                return;
            };
            format!(
                r#"(() => {{
const state = window.__semanticDxEditorProtocol?.get({session_id});
if (!state || state.protocol_version !== {} || state.schema_fingerprint !== {schema_fingerprint}
    || state.external_revision >= {}) return;
state.external_revision = {};
window.__semanticDxEditor?.replaceDocument({session_id}, {document}, {history_policy});
}})();"#,
                session.protocol_version, session.external_revision.0, session.external_revision.0
            )
        }
        EngineCommand::Acknowledge {
            session,
            local_revision,
        } => {
            let Ok(session_id) = serde_json::to_string(&session.session_id) else {
                return;
            };
            let Ok(schema_fingerprint) = serde_json::to_string(&session.schema_fingerprint) else {
                return;
            };
            format!(
                r#"(() => {{
const state = window.__semanticDxEditorProtocol?.get({session_id});
if (!state || state.protocol_version !== {} || state.schema_fingerprint !== {schema_fingerprint}
    || state.lastLocalRevision < {} || state.external_revision > {}) return;
state.external_revision = {};
}})();"#,
                session.protocol_version,
                local_revision.0,
                session.external_revision.0,
                session.external_revision.0
            )
        }
        EngineCommand::MentionSuggestions {
            session,
            request_id,
            suggestions,
        } => {
            let Ok(session_id) = serde_json::to_string(&session.session_id) else {
                return;
            };
            let Ok(schema_fingerprint) = serde_json::to_string(&session.schema_fingerprint) else {
                return;
            };
            let Ok(suggestions) = serde_json::to_string(suggestions) else {
                return;
            };
            format!(
                r#"(() => {{
const state = window.__semanticDxEditorProtocol?.get({session_id});
if (!state || state.protocol_version !== {} || state.schema_fingerprint !== {schema_fingerprint}
    || state.external_revision !== {}) return;
const requests = window.__semanticDxEditorMentionRequests?.get({session_id});
const resolve = requests?.get({request_id});
if (!resolve) return;
requests.delete({request_id});
resolve({suggestions});
}})();"#,
                session.protocol_version, session.external_revision.0
            )
        }
        EngineCommand::EntitySearchResults {
            session,
            request_id,
            candidates,
        } => entity_response_script(session, *request_id, candidates),
        EngineCommand::EntityPreviewResult {
            session,
            request_id,
            preview,
        } => entity_response_script(session, *request_id, preview),
        EngineCommand::Destroy { session } => {
            let Ok(session_id) = serde_json::to_string(&session.session_id) else {
                return;
            };
            format!(
                "window.__semanticDxEditor?.destroy({session_id}); window.__semanticDxEditorProtocol?.delete({session_id}); window.__semanticDxEditorMentionRequests?.delete({session_id}); window.__semanticDxEditorEntityRequests?.delete({session_id});"
            )
        }
        EngineCommand::Mount { .. } => return,
    };
    let _ = document::eval(&script);
}

fn entity_response_script(
    session: &ProtocolSession,
    request_id: u64,
    value: &impl serde::Serialize,
) -> String {
    let Ok(session_id) = serde_json::to_string(&session.session_id) else {
        return String::new();
    };
    let Ok(schema_fingerprint) = serde_json::to_string(&session.schema_fingerprint) else {
        return String::new();
    };
    let Ok(value) = serde_json::to_string(value) else {
        return String::new();
    };
    format!(
        r#"(() => {{
const state = window.__semanticDxEditorProtocol?.get({session_id});
if (!state || state.protocol_version !== {} || state.schema_fingerprint !== {schema_fingerprint}
    || state.external_revision !== {}) return;
const requests = window.__semanticDxEditorEntityRequests?.get({session_id});
const resolve = requests?.get({request_id});
if (!resolve) return;
requests.delete({request_id});
resolve({value});
}})();"#,
        session.protocol_version, session.external_revision.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{EDITOR_PROTOCOL_VERSION, Revision};

    fn session() -> ProtocolSession {
        ProtocolSession::new("session-1", "schema-1", Revision(3))
    }

    #[test]
    fn bridge_document_event_uses_v2_without_projection() {
        let event = serde_json::json!({
            "kind": "documentChange",
            "protocol_version": EDITOR_PROTOCOL_VERSION,
            "session_id": "session-1",
            "schema_fingerprint": "schema-1",
            "external_revision": 3,
            "revision": 4,
            "document": ComponentDocumentV2::plain_text("updated"),
        });
        let event = serde_json::from_value::<EngineEvent>(event).unwrap();
        assert!(matches!(
            event,
            EngineEvent::DocumentChange { session, revision: Revision(4), document }
                if session == self::session() && document.version == 2
        ));
    }

    #[test]
    fn bridge_rejects_unversioned_and_unknown_events() {
        let unversioned = serde_json::json!({
            "kind": "ready",
            "session_id": "session-1",
        });
        assert!(serde_json::from_value::<EngineEvent>(unversioned).is_err());

        let unknown = serde_json::json!({
            "kind": "futureEvent",
            "protocol_version": EDITOR_PROTOCOL_VERSION,
            "session_id": "session-1",
            "schema_fingerprint": "schema-1",
            "external_revision": 3,
        });
        assert!(serde_json::from_value::<EngineEvent>(unknown).is_err());
    }

    #[test]
    fn entity_responses_are_scoped_to_the_request_and_session() {
        let script = entity_response_script(
            &session(),
            17,
            &vec![crate::entity_link::EntityLinkCandidate {
                id: "person-1".to_string(),
                label: "Ada".to_string(),
                detail: None,
            }],
        );
        assert!(script.contains("schema-1"));
        assert!(script.contains("requests?.get(17)"));
        assert!(script.contains("person-1"));
    }
}
