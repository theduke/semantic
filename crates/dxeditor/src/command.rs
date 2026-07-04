use std::{collections::BTreeMap, rc::Rc};

use serde_json::Value;

use crate::{
    EditorError,
    document::{
        BlockNode, COMPONENT_PARAGRAPH, InlineNode, MARK_BOLD, MARK_CODE, MARK_ITALIC, Mark, NodeId,
    },
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
        "editor.set_block_type",
        Rc::new(|ctx, args| {
            let document = ctx.state.document();
            let component = args
                .get("component")
                .and_then(Value::as_str)
                .ok_or_else(|| EditorError::Message("missing component".to_string()))?
                .to_string();
            let attrs = args
                .get("attrs")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .map(NodeId::from)
                .or_else(|| document.blocks.first().map(|block| block.id.clone()))
                .ok_or_else(|| EditorError::Message("missing block id".to_string()))?;
            Ok(Transaction::new(vec![Operation::SetBlockType {
                id,
                component,
                attrs,
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
        "editor.set_block_text",
        Rc::new(|_ctx, args| {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .map(NodeId::from)
                .ok_or_else(|| EditorError::Message("missing block id".to_string()))?;
            let text = args
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| EditorError::Message("missing text".to_string()))?
                .to_string();
            Ok(Transaction::new(vec![Operation::SetInlineText {
                block_id: id,
                text,
            }]))
        }),
    );

    registry.register(
        "editor.set_block_inline_content",
        Rc::new(|_ctx, args| {
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .map(NodeId::from)
                .ok_or_else(|| EditorError::Message("missing block id".to_string()))?;
            let inline = args
                .get("inline")
                .cloned()
                .map(serde_json::from_value::<Vec<InlineNode>>)
                .transpose()
                .map_err(|err| EditorError::InvalidPayload {
                    format: "dxeditor.inline.v1".to_string(),
                    message: err.to_string(),
                })?
                .ok_or_else(|| EditorError::Message("missing inline content".to_string()))?;
            Ok(Transaction::new(vec![Operation::SetInlineContent {
                block_id: id,
                inline,
            }]))
        }),
    );

    registry.register(
        "editor.toggle_mark",
        Rc::new(|ctx, args| {
            let mark_name = args
                .get("mark")
                .and_then(Value::as_str)
                .ok_or_else(|| EditorError::Message("missing mark".to_string()))?;
            let mark = match mark_name {
                MARK_BOLD | MARK_ITALIC | MARK_CODE => Mark::new(mark_name),
                other => return Err(EditorError::Message(format!("unknown mark '{other}'"))),
            };
            let selection = args
                .get("selection")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|err| EditorError::InvalidPayload {
                    format: "dxeditor.selection.v1".to_string(),
                    message: err.to_string(),
                })?
                .or_else(|| ctx.state.selection());
            let Some(selection) = selection else {
                return Ok(Transaction::empty());
            };
            Ok(Transaction::new(vec![Operation::ToggleMark {
                selection,
                mark,
            }]))
        }),
    );

    registry.register(
        "editor.split_block",
        Rc::new(|ctx, args| {
            let document = ctx.state.document();
            let id = args
                .get("id")
                .and_then(Value::as_str)
                .map(NodeId::from)
                .or_else(|| document.blocks.first().map(|block| block.id.clone()))
                .ok_or_else(|| EditorError::Message("missing block id".to_string()))?;
            let offset = args
                .get("offset")
                .and_then(Value::as_u64)
                .map(|value| value as usize)
                .unwrap_or_else(|| {
                    ctx.state
                        .selection()
                        .filter(|selection| selection.is_collapsed())
                        .map(|selection| selection.anchor.offset)
                        .unwrap_or_else(|| {
                            document
                                .blocks
                                .iter()
                                .find(|block| block.id == id)
                                .map(BlockNode::text_content)
                                .map(|text| text.chars().count())
                                .unwrap_or_default()
                        })
                });
            Ok(Transaction::new(vec![Operation::SplitBlock { id, offset }]))
        }),
    );

    registry.register(
        "editor.merge_blocks",
        Rc::new(|ctx, args| {
            let document = ctx.state.document();
            let first_id = args
                .get("first_id")
                .and_then(Value::as_str)
                .map(NodeId::from);
            let second_id = args
                .get("second_id")
                .and_then(Value::as_str)
                .map(NodeId::from);
            let (first_id, second_id) = first_id
                .zip(second_id)
                .or_else(|| {
                    let index = args.get("index").and_then(Value::as_u64)? as usize;
                    Some((
                        document.blocks.get(index)?.id.clone(),
                        document.blocks.get(index + 1)?.id.clone(),
                    ))
                })
                .or_else(|| {
                    Some((
                        document.blocks.first()?.id.clone(),
                        document.blocks.get(1)?.id.clone(),
                    ))
                })
                .ok_or_else(|| EditorError::Message("missing adjacent block ids".to_string()))?;
            Ok(Transaction::new(vec![Operation::MergeBlocks {
                first_id,
                second_id,
            }]))
        }),
    );

    registry.register(
        "editor.noop",
        Rc::new(|_ctx, _args| Ok(Transaction::empty())),
    );
}
