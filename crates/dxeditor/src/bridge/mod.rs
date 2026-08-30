use dioxus::prelude::*;

use crate::{
    document_v2::ComponentDocumentV2,
    protocol::{EngineCommand, EngineEvent, ProtocolSession, SessionRevisionGuard},
};

const ENGINE_SCRIPT: &str = include_str!("../../web/dist/editor.iife.js");

#[component]
pub(crate) fn EditorBridge(
    editor_id: String,
    session: ProtocolSession,
    document_value: ComponentDocumentV2,
    readonly: bool,
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
        let script = format!(
            r#"
{ENGINE_SCRIPT}
const editorId = {editor_id_json};
const protocol = {session_json};
const sessionId = protocol.session_id;
window.__semanticDxEditorProtocol ??= new Map();
window.__semanticDxEditorMentionRequests ??= new Map();
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
        spawn(async move {
            let mut guard = SessionRevisionGuard::new(guard_session);
            let mut eval = document::eval(&script);
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
            session, document, ..
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
            format!(
                r#"(() => {{
const state = window.__semanticDxEditorProtocol?.get({session_id});
if (!state || state.protocol_version !== {} || state.schema_fingerprint !== {schema_fingerprint}
    || state.external_revision >= {}) return;
state.external_revision = {};
window.__semanticDxEditor?.replaceDocument({session_id}, {document});
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
        EngineCommand::Destroy { session } => {
            let Ok(session_id) = serde_json::to_string(&session.session_id) else {
                return;
            };
            format!(
                "window.__semanticDxEditor?.destroy({session_id}); window.__semanticDxEditorProtocol?.delete({session_id}); window.__semanticDxEditorMentionRequests?.delete({session_id});"
            )
        }
        EngineCommand::Mount { .. } => return,
    };
    let _ = document::eval(&script);
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
}
