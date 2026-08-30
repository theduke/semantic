pub mod action;
mod bridge;
pub mod catalog;
pub mod codec;
pub mod command;
pub mod component;
pub mod component_spec;
pub mod document;
pub mod document_v2;
pub mod extension;
pub mod format;
pub mod input;
#[cfg(feature = "markdown")]
pub mod markdown;
pub mod migrate;
pub mod protocol;
pub mod render;
pub mod selection;
pub mod state;
pub mod suggestion;
pub mod transaction;

pub use action::{
    ActionHandler, ActionPredicate, ActionRegistry, ActionSurface, EditorAction,
    EditorActionContext, KeyBinding,
};
pub use catalog::{EditorCatalog, EditorCatalogBuilder, EditorCatalogConfig};
pub use codec::{
    CodecRegistry, DecodeContext, EditorCodec, EditorPayload, EncodeContext,
    register_standard_codecs,
};
pub use command::{CommandContext, CommandHandler, CommandRegistry};
pub use component::{
    ComponentRegistry, DocumentEditor, DocumentView, Editor, EditorActionsMode,
    EditorComponentKind, EditorComponentRegistration, MarkdownEditor, PlainTextEditor,
};
pub use component_spec::{
    AttributeSpec, AttributeType, ClipboardPolicy, ComponentBehavior, ComponentCatalog,
    ComponentCatalogError, ComponentKind, ComponentSpec, ContentRule, DomDescriptor,
    FORMAT_MARKDOWN, FORMAT_PLAIN_TEXT, FORMAT_TYPED_DOCUMENT, FormatCapability, IdentityPolicy,
    PlainTextFallback, is_safe_url, register_standard_component_specs, validate_component_document,
};
pub use document::{
    BlockNode, EditorDocument, InlineNode, Mark, NodeContent, NodeId, TableCell, TableNode,
    TableRow,
};
pub use document_v2::{
    AttributeMap, COMPONENT_DOCUMENT_FORMAT, COMPONENT_DOCUMENT_SCHEMA, COMPONENT_DOCUMENT_VERSION,
    CodeBlockAttributes, ComponentDocumentV2, ComponentId, ComponentMark, ComponentNode,
    DocumentMetadata, DocumentSchemaId, DocumentValidationError, HeadingAttributes,
    ImageAttributes, LinkAttributes, MentionAttributes, NormalizationOptions,
    OpaqueMarkdownAttributes, OrderedListAttributes, TableAlignment, TableCellAttributes,
    TaskItemAttributes, UnknownComponentAttributes, UnknownComponentPolicy, ValidationIssue,
    ValidationLimits, normalize_component_document,
};
pub use extension::EditorExtension;
pub use format::{
    DecodeOptions, DecodedDocument, DiagnosticSeverity, DocumentFormat, DocumentFormatRegistry,
    EncodeOptions, EncodedPayload, Fidelity, FormatDescriptor, FormatDiagnostic, FormatError,
    FormatId, FormatSourceState, LegacyV1DocumentFormat, PlainTextDocumentFormat, SourceRange,
    TYPED_DOCUMENT_JSON_SCHEMA, TypedDocumentCodec, register_standard_document_formats,
};
pub use input::{InputEvent, reconcile_block_text, transaction_for_event};
pub use migrate::{
    MigrationError, MigrationOptions, migrate_v1_to_v2, migrate_v1_to_v2_with, migrate_v2_to_v1,
};
pub use protocol::{
    EDITOR_PROTOCOL_VERSION, EngineCommand, EngineEvent, HistoryPolicy, MentionSuggestion,
    ProtocolError, ProtocolSession, Revision, SessionRevisionGuard,
};
pub use render::{
    ChromeRenderer, ChromeRendererContext, ComponentRenderKind, ComponentRenderer,
    ComponentRendererContext, EditorRenderRegistry,
};
pub use selection::{EditorSelection, TextPosition};
pub use state::{EditorHandle, EditorHistory, EditorState};
pub use suggestion::{
    InputRule, InputRuleRegistry, SuggestionItem, SuggestionProvider, SuggestionQueryContext,
    SuggestionRegistry,
};
pub use transaction::{Operation, Transaction};

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error("unknown codec format '{0}'")]
    UnknownFormat(String),
    #[error("invalid payload for format '{format}': {message}")]
    InvalidPayload { format: String, message: String },
    #[error("unknown command '{0}'")]
    UnknownCommand(String),
    #[error("unknown action '{0}'")]
    UnknownAction(String),
    #[error("transaction failed: {0}")]
    Transaction(String),
    #[error("{0}")]
    Message(String),
}
