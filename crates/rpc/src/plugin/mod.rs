//! Host transport adapters for the shared interface session.
#[cfg(all(feature = "plugin-stdio", not(target_arch = "wasm32")))]
pub mod stdio;
#[cfg(all(feature = "plugin-ws", not(target_arch = "wasm32")))]
pub mod websocket;

use crate::interface::{
    ImplementationDescriptor, InterfaceImplementation, InvocationError, session::Session,
};
use semantic_data::value::Value;
use semantic_rpc_core::interface_protocol::{CODEC, InterfaceMessage, PROFILE, PROTOCOL_VERSION};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::mpsc;

/// How the exports declared by the two handshake peers must relate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportMatch {
    /// Both peers declare exactly the same exports. Used for plugins, whose
    /// contract is fixed by the host.
    Exact,
    /// The connecting peer declares only the exports it requires, and the
    /// accepting peer must provide each of them unchanged. It may provide
    /// more, which the connecting peer learns from the accepting peer's reply.
    RequiredSubset,
}

impl ExportMatch {
    /// Whether the `required` exports of the connecting peer are satisfied by
    /// the `provided` exports of the accepting peer.
    fn satisfied(
        self,
        required: &[ImplementationDescriptor],
        provided: &[ImplementationDescriptor],
    ) -> bool {
        match self {
            Self::Exact => required == provided,
            Self::RequiredSubset => required
                .iter()
                .all(|descriptor| provided.contains(descriptor)),
        }
    }
}

pub struct ProviderConnection {
    pub implementation: Arc<dyn InterfaceImplementation>,
    session: Session,
    cleanup: Pin<Box<dyn Future<Output = Result<(), InvocationError>> + Send>>,
}

impl ProviderConnection {
    /// Observe transport termination independently of calls and cleanup ownership.
    pub fn closed(&self) -> impl Future<Output = ()> + Send + 'static {
        let session = self.session.clone();
        async move { session.closed().await }
    }
    #[cfg(all(feature = "client-http-native", not(target_arch = "wasm32")))]
    pub(crate) fn into_session(self) -> Session {
        self.session
    }
    pub async fn shutdown(self) -> Result<(), InvocationError> {
        let result = self.session.shutdown().await;
        self.cleanup.await?;
        result
    }
}

fn error(message: impl ToString) -> InvocationError {
    InvocationError::new("connection_lost", message.to_string())
}

pub(crate) async fn negotiate(
    incoming: &mut mpsc::UnboundedReceiver<Result<InterfaceMessage, InvocationError>>,
    outgoing: &mpsc::UnboundedSender<InterfaceMessage>,
    exports: &[ImplementationDescriptor],
    revision: Option<String>,
    configuration: Value,
    export_match: ExportMatch,
) -> Result<Vec<ImplementationDescriptor>, InvocationError> {
    outgoing
        .send(InterfaceMessage::Hello {
            version: PROTOCOL_VERSION,
            profile: PROFILE.into(),
            codec: CODEC.into(),
            revision: revision.clone(),
            exports: exports.to_vec(),
        })
        .map_err(error)?;
    let peer_exports = match incoming
        .recv()
        .await
        .ok_or_else(|| error("disconnected during handshake"))??
    {
        InterfaceMessage::Hello {
            version,
            profile,
            codec,
            revision: peer_revision,
            exports: peer_exports,
        } if version == PROTOCOL_VERSION
            && profile == PROFILE
            && codec == CODEC
            && peer_revision == revision
            && export_match.satisfied(exports, &peer_exports) =>
        {
            peer_exports
        }
        _ => {
            return Err(InvocationError::new(
                "interface_incompatible",
                "peer handshake does not match authoritative exports/profile/revision",
            ));
        }
    };
    outgoing
        .send(InterfaceMessage::Configure { configuration })
        .map_err(error)?;
    match incoming
        .recv()
        .await
        .ok_or_else(|| error("disconnected before ready"))??
    {
        InterfaceMessage::Ready => Ok(peer_exports),
        _ => Err(InvocationError::new(
            "protocol_violation",
            "expected readiness acknowledgement",
        )),
    }
}

/// SDK peer bootstrap shared by executable and WebSocket service adapters.
/// Configuration is delivered only after the host and peer declarations agree
/// according to `export_match`.
pub async fn accept<F, Fut>(
    mut incoming: mpsc::UnboundedReceiver<Result<InterfaceMessage, InvocationError>>,
    outgoing: mpsc::UnboundedSender<InterfaceMessage>,
    implementation: Arc<dyn InterfaceImplementation>,
    revision: Option<String>,
    export_match: ExportMatch,
    configure: F,
) -> Result<Session, InvocationError>
where
    F: FnOnce(Value) -> Fut,
    Fut: Future<Output = Result<(), InvocationError>>,
{
    let exports = implementation.descriptors().to_vec();
    match incoming
        .recv()
        .await
        .ok_or_else(|| error("disconnected during handshake"))??
    {
        InterfaceMessage::Hello {
            version,
            profile,
            codec,
            revision: peer_revision,
            exports: peer_exports,
        } if version == PROTOCOL_VERSION
            && profile == PROFILE
            && codec == CODEC
            && peer_revision == revision
            && export_match.satisfied(&peer_exports, &exports) => {}
        _ => {
            return Err(InvocationError::new(
                "interface_incompatible",
                "host handshake does not match implementation",
            ));
        }
    }
    outgoing
        .send(InterfaceMessage::Hello {
            version: PROTOCOL_VERSION,
            profile: PROFILE.into(),
            codec: CODEC.into(),
            revision,
            exports: exports.clone(),
        })
        .map_err(error)?;
    let configuration = match incoming
        .recv()
        .await
        .ok_or_else(|| error("disconnected before configuration"))??
    {
        InterfaceMessage::Configure { configuration } => configuration,
        _ => {
            return Err(InvocationError::new(
                "protocol_violation",
                "expected configuration",
            ));
        }
    };
    configure(configuration).await?;
    outgoing.send(InterfaceMessage::Ready).map_err(error)?;
    Ok(Session::start(
        incoming,
        outgoing,
        Some(implementation),
        exports,
    ))
}

#[cfg(test)]
mod export_match_tests {
    use super::*;
    use crate::interface::InterfaceRef;

    fn descriptor(export: &str, fingerprint: &str) -> ImplementationDescriptor {
        ImplementationDescriptor {
            export: export.into(),
            interface: InterfaceRef {
                package: "test".into(),
                module: "v1".into(),
                contract: None,
                name: "Test".into(),
            },
            package_version: "1".into(),
            fingerprint: fingerprint.into(),
        }
    }

    #[test]
    fn exact_requires_identical_exports() {
        let a = descriptor("a", "1");
        let b = descriptor("b", "1");
        assert!(ExportMatch::Exact.satisfied(&[a.clone()], &[a.clone()]));
        assert!(!ExportMatch::Exact.satisfied(&[a.clone()], &[a.clone(), b.clone()]));
        assert!(!ExportMatch::Exact.satisfied(&[a.clone(), b], &[a]));
    }

    #[test]
    fn required_subset_allows_extra_provided_exports() {
        let a = descriptor("a", "1");
        let b = descriptor("b", "1");
        assert!(ExportMatch::RequiredSubset.satisfied(&[a.clone()], &[a.clone(), b.clone()]));
        assert!(ExportMatch::RequiredSubset.satisfied(&[], &[a.clone()]));
        assert!(!ExportMatch::RequiredSubset.satisfied(&[a.clone(), b], &[a.clone()]));
        // A required export must match unchanged, fingerprint included.
        assert!(!ExportMatch::RequiredSubset.satisfied(&[a], &[descriptor("a", "2")]));
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;

    #[tokio::test]
    async fn disconnected_shutdown_preserves_cleanup_errors() {
        for fail in [false, true] {
            let (incoming, receiver) = mpsc::unbounded_channel();
            let (sender, _outgoing) = mpsc::unbounded_channel();
            let session = Session::start(receiver, sender, None, vec![]);
            drop(incoming);
            session.closed().await;
            let connection = ProviderConnection {
                implementation: Arc::new(session.clone()),
                session,
                cleanup: Box::pin(async move {
                    if fail {
                        Err(InvocationError::new("cleanup_failed", "cleanup failed"))
                    } else {
                        Ok(())
                    }
                }),
            };
            let result = connection.shutdown().await;
            if fail {
                assert_eq!(result.unwrap_err().code, "cleanup_failed");
            } else {
                result.unwrap();
            }
        }
    }
}
