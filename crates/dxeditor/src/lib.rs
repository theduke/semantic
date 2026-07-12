pub mod action;
mod bridge;
pub mod catalog;
pub mod codec;
pub mod command;
pub mod component;
pub mod document;
pub mod extension;
pub mod input;
#[cfg(feature = "markdown")]
pub mod markdown;
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
    ComponentRegistry, DocumentEditor, DocumentView, Editor, EditorComponentKind,
    EditorComponentRegistration, MarkdownEditor, PlainTextEditor,
};
pub use document::{
    BlockNode, EditorDocument, InlineNode, Mark, NodeContent, NodeId, TableCell, TableNode,
    TableRow,
};
pub use extension::EditorExtension;
pub use input::{InputEvent, reconcile_block_text, transaction_for_event};
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
