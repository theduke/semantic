use std::{collections::BTreeMap, rc::Rc};

use serde_json::Value;

use crate::{
    EditorError,
    document::{BlockNode, COMPONENT_PARAGRAPH, NodeId},
    state::EditorState,
    transaction::{Operation, Transaction},
};

#[derive(Clone)]
pub struct CommandContext {
    pub state: EditorState,
}

pub type CommandHandler = Rc<dyn Fn(CommandContext, Value) -> Result<Transaction, EditorError>>;

#[derive(Clone, Default)]
pub struct CommandRegistry {
    commands: BTreeMap<String, CommandHandler>,
}

impl CommandRegistry {
    pub fn register(&mut self, id: impl Into<String>, handler: CommandHandler) {
        self.commands.insert(id.into(), handler);
    }

    pub fn command(&self, id: &str) -> Option<CommandHandler> {
        self.commands.get(id).cloned()
    }

    pub fn dispatch(
        &self,
        id: &str,
        state: &EditorState,
        args: Value,
    ) -> Result<Transaction, EditorError> {
        let command = self
            .command(id)
            .ok_or_else(|| EditorError::UnknownCommand(id.to_string()))?;
        command(
            CommandContext {
                state: state.clone(),
            },
            args,
        )
    }
}

pub fn register_standard_commands(registry: &mut CommandRegistry) {
    registry.register(
        "editor.replace_document",
        Rc::new(|_ctx, args| {
            let document =
                serde_json::from_value(args).map_err(|err| EditorError::InvalidPayload {
                    format: "dxeditor.document.v1".to_string(),
                    message: err.to_string(),
                })?;
            Ok(Transaction::new(vec![Operation::ReplaceDocument(document)]))
        }),
    );

    registry.register(
        "editor.insert_block",
        Rc::new(|ctx, args| {
            let document = ctx.state.document();
            let component = args
                .get("component")
                .and_then(Value::as_str)
                .unwrap_or(COMPONENT_PARAGRAPH)
                .to_string();
            let index = args
                .get("index")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or(document.blocks.len());
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("block-{}", document.blocks.len() + 1));
            let block = BlockNode::new(
                id,
                component,
                serde_json::Map::new(),
                crate::document::NodeContent::Inline(Vec::new()),
            );
            Ok(Transaction::new(vec![Operation::InsertBlock {
                index,
                block,
            }]))
        }),
    );

    registry.register(
        "editor.set_block_component",
        Rc::new(|ctx, args| {
            let document = ctx.state.document();
            let component = args
                .get("component")
                .and_then(Value::as_str)
                .ok_or_else(|| EditorError::Message("missing component".to_string()))?
                .to_string();
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .map(NodeId::from)
                .or_else(|| document.blocks.first().map(|block| block.id.clone()))
                .ok_or_else(|| EditorError::Message("missing block id".to_string()))?;
            Ok(Transaction::new(vec![Operation::SetBlockComponent {
                id,
                component,
            }]))
        }),
    );

    registry.register(
        "editor.set_plain_text",
        Rc::new(|ctx, args| {
            let text = args
                .as_str()
                .ok_or_else(|| EditorError::Message("expected text string".to_string()))?
                .to_string();
            let id = ctx
                .state
                .document()
                .blocks
                .first()
                .map(|block| block.id.clone())
                .unwrap_or_else(|| NodeId::from("block-1"));
            Ok(Transaction::new(vec![Operation::SetInlineText {
                block_id: id,
                text,
            }]))
        }),
    );

    registry.register(
        "editor.noop",
        Rc::new(|_ctx, _args| Ok(Transaction::empty())),
    );
}
