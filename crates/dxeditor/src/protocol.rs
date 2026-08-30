use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::document_v2::ComponentDocumentV2;

pub const EDITOR_PROTOCOL_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(pub u64);

impl Revision {
    pub const ZERO: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl std::fmt::Display for Revision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolSession {
    pub protocol_version: u32,
    pub session_id: String,
    pub schema_fingerprint: String,
    pub external_revision: Revision,
}

impl ProtocolSession {
    pub fn new(
        session_id: impl Into<String>,
        schema_fingerprint: impl Into<String>,
        external_revision: Revision,
    ) -> Self {
        Self {
            protocol_version: EDITOR_PROTOCOL_VERSION,
            session_id: session_id.into(),
            schema_fingerprint: schema_fingerprint.into(),
            external_revision,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryPolicy {
    Preserve,
    #[default]
    Reset,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineCommand {
    Mount {
        #[serde(flatten)]
        session: ProtocolSession,
        document: ComponentDocumentV2,
        readonly: bool,
    },
    ReplaceDocument {
        #[serde(flatten)]
        session: ProtocolSession,
        document: ComponentDocumentV2,
        history_policy: HistoryPolicy,
    },
    Acknowledge {
        #[serde(flatten)]
        session: ProtocolSession,
        local_revision: Revision,
    },
    RunCommand {
        #[serde(flatten)]
        session: ProtocolSession,
        request_id: u64,
        expected_revision: Revision,
        command: String,
        #[serde(default)]
        args: Value,
    },
    MentionSuggestions {
        #[serde(flatten)]
        session: ProtocolSession,
        request_id: u64,
        suggestions: Vec<MentionSuggestion>,
    },
    Destroy {
        #[serde(flatten)]
        session: ProtocolSession,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineEvent {
    Ready {
        #[serde(flatten)]
        session: ProtocolSession,
    },
    DocumentChange {
        #[serde(flatten)]
        session: ProtocolSession,
        revision: Revision,
        document: ComponentDocumentV2,
    },
    Blur {
        #[serde(flatten)]
        session: ProtocolSession,
        revision: Revision,
        document: ComponentDocumentV2,
    },
    MentionQuery {
        #[serde(flatten)]
        session: ProtocolSession,
        request_id: u64,
        query: String,
    },
    Error {
        #[serde(flatten)]
        session: ProtocolSession,
        message: String,
    },
}

impl EngineEvent {
    pub fn session(&self) -> &ProtocolSession {
        match self {
            Self::Ready { session }
            | Self::DocumentChange { session, .. }
            | Self::Blur { session, .. }
            | Self::MentionQuery { session, .. }
            | Self::Error { session, .. } => session,
        }
    }

    pub fn revision(&self) -> Option<Revision> {
        match self {
            Self::DocumentChange { revision, .. } | Self::Blur { revision, .. } => Some(*revision),
            Self::Ready { .. } | Self::MentionQuery { .. } | Self::Error { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    #[error("unsupported editor protocol version {actual}; expected {expected}")]
    Version { expected: u32, actual: u32 },
    #[error("stale editor session '{actual}'; expected '{expected}'")]
    Session { expected: String, actual: String },
    #[error("editor schema fingerprint mismatch")]
    SchemaFingerprint,
    #[error("stale external revision {actual}; expected at least {expected}")]
    ExternalRevision {
        expected: Revision,
        actual: Revision,
    },
    #[error("stale local revision {actual}; last accepted revision is {last_accepted}")]
    LocalRevision {
        last_accepted: Revision,
        actual: Revision,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRevisionGuard {
    expected: ProtocolSession,
    last_local_revision: Revision,
}

impl SessionRevisionGuard {
    pub fn new(expected: ProtocolSession) -> Self {
        Self {
            expected,
            last_local_revision: Revision::ZERO,
        }
    }

    pub fn expected(&self) -> &ProtocolSession {
        &self.expected
    }

    pub fn last_local_revision(&self) -> Revision {
        self.last_local_revision
    }

    pub fn advance_external_revision(
        &mut self,
        external_revision: Revision,
    ) -> Result<(), ProtocolError> {
        if external_revision < self.expected.external_revision {
            return Err(ProtocolError::ExternalRevision {
                expected: self.expected.external_revision,
                actual: external_revision,
            });
        }
        self.expected.external_revision = external_revision;
        Ok(())
    }

    pub fn accept(&mut self, event: &EngineEvent) -> Result<(), ProtocolError> {
        let session = event.session();
        self.validate_session(session)?;

        match event {
            EngineEvent::DocumentChange { revision, .. } => {
                if *revision <= self.last_local_revision {
                    return Err(ProtocolError::LocalRevision {
                        last_accepted: self.last_local_revision,
                        actual: *revision,
                    });
                }
                self.last_local_revision = *revision;
            }
            EngineEvent::Blur { revision, .. } => {
                if *revision < self.last_local_revision {
                    return Err(ProtocolError::LocalRevision {
                        last_accepted: self.last_local_revision,
                        actual: *revision,
                    });
                }
                self.last_local_revision = *revision;
            }
            EngineEvent::Ready { .. }
            | EngineEvent::MentionQuery { .. }
            | EngineEvent::Error { .. } => {}
        }
        Ok(())
    }

    fn validate_session(&mut self, session: &ProtocolSession) -> Result<(), ProtocolError> {
        if session.protocol_version != EDITOR_PROTOCOL_VERSION {
            return Err(ProtocolError::Version {
                expected: EDITOR_PROTOCOL_VERSION,
                actual: session.protocol_version,
            });
        }
        if session.session_id != self.expected.session_id {
            return Err(ProtocolError::Session {
                expected: self.expected.session_id.clone(),
                actual: session.session_id.clone(),
            });
        }
        if session.schema_fingerprint != self.expected.schema_fingerprint {
            return Err(ProtocolError::SchemaFingerprint);
        }
        if session.external_revision < self.expected.external_revision {
            return Err(ProtocolError::ExternalRevision {
                expected: self.expected.external_revision,
                actual: session.external_revision,
            });
        }
        self.expected.external_revision = session.external_revision;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MentionSuggestion {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(external_revision: u64) -> ProtocolSession {
        ProtocolSession::new("session-1", "schema-1", Revision(external_revision))
    }

    #[test]
    fn commands_serialize_versioned_v2_documents() {
        let command = EngineCommand::Mount {
            session: session(7),
            document: ComponentDocumentV2::plain_text("hello"),
            readonly: false,
        };
        let value = serde_json::to_value(command).unwrap();
        assert_eq!(value["kind"], "mount");
        assert_eq!(value["protocol_version"], EDITOR_PROTOCOL_VERSION);
        assert_eq!(value["external_revision"], 7);
        assert_eq!(value["document"]["version"], 2);
    }

    #[test]
    fn guard_rejects_stale_local_and_external_revisions() {
        let mut guard = SessionRevisionGuard::new(session(3));
        let event = EngineEvent::DocumentChange {
            session: session(3),
            revision: Revision(4),
            document: ComponentDocumentV2::plain_text("four"),
        };
        guard.accept(&event).unwrap();
        assert!(matches!(
            guard.accept(&event),
            Err(ProtocolError::LocalRevision { .. })
        ));

        let stale = EngineEvent::Blur {
            session: session(2),
            revision: Revision(4),
            document: ComponentDocumentV2::plain_text("stale"),
        };
        assert!(matches!(
            guard.accept(&stale),
            Err(ProtocolError::ExternalRevision { .. })
        ));
    }

    #[test]
    fn blur_may_flush_the_latest_accepted_revision() {
        let mut guard = SessionRevisionGuard::new(session(0));
        guard
            .accept(&EngineEvent::DocumentChange {
                session: session(0),
                revision: Revision(1),
                document: ComponentDocumentV2::plain_text("change"),
            })
            .unwrap();
        guard
            .accept(&EngineEvent::Blur {
                session: session(0),
                revision: Revision(1),
                document: ComponentDocumentV2::plain_text("change"),
            })
            .unwrap();
    }

    #[test]
    fn mention_queries_and_responses_are_versioned_session_messages() {
        let event = EngineEvent::MentionQuery {
            session: session(4),
            request_id: 9,
            query: "ali".to_string(),
        };
        let event_value = serde_json::to_value(&event).unwrap();
        assert_eq!(event_value["kind"], "mentionQuery");
        assert_eq!(event_value["protocol_version"], EDITOR_PROTOCOL_VERSION);

        let command = EngineCommand::MentionSuggestions {
            session: session(4),
            request_id: 9,
            suggestions: vec![MentionSuggestion {
                id: "entity-alice".to_string(),
                label: "Alice".to_string(),
                detail: Some("Person".to_string()),
            }],
        };
        let command_value = serde_json::to_value(command).unwrap();
        assert_eq!(command_value["kind"], "mentionSuggestions");
        assert_eq!(command_value["suggestions"][0]["label"], "Alice");
    }
}
