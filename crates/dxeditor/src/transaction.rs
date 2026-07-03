use crate::{
    EditorError,
    document::{BlockNode, EditorDocument, NodeContent, NodeId},
    selection::EditorSelection,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Transaction {
    pub operations: Vec<Operation>,
    pub add_to_history: bool,
}

impl Transaction {
    pub fn new(operations: Vec<Operation>) -> Self {
        Self {
            operations,
            add_to_history: true,
        }
    }

    pub fn empty() -> Self {
        Self {
            operations: Vec::new(),
            add_to_history: false,
        }
    }

    pub fn apply(
        &self,
        document: &mut EditorDocument,
        selection: &mut Option<EditorSelection>,
    ) -> Result<(), EditorError> {
        for operation in &self.operations {
            operation.apply(document, selection)?;
        }
        normalize_document(document);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    ReplaceDocument(EditorDocument),
    InsertBlock { index: usize, block: BlockNode },
    ReplaceBlock { id: NodeId, block: BlockNode },
    RemoveBlock { id: NodeId },
    SetBlockComponent { id: NodeId, component: String },
    SetInlineText { block_id: NodeId, text: String },
    SetSelection(Option<EditorSelection>),
}

impl Operation {
    fn apply(
        &self,
        document: &mut EditorDocument,
        selection: &mut Option<EditorSelection>,
    ) -> Result<(), EditorError> {
        match self {
            Operation::ReplaceDocument(next) => {
                *document = next.clone();
            }
            Operation::InsertBlock { index, block } => {
                let index = (*index).min(document.blocks.len());
                document.blocks.insert(index, block.clone());
            }
            Operation::ReplaceBlock { id, block } => {
                let Some(existing) = document.blocks.iter_mut().find(|node| node.id == *id) else {
                    return Err(EditorError::Transaction(format!(
                        "block '{}' does not exist",
                        id.0
                    )));
                };
                *existing = block.clone();
            }
            Operation::RemoveBlock { id } => {
                document.blocks.retain(|node| node.id != *id);
            }
            Operation::SetBlockComponent { id, component } => {
                let Some(existing) = document.blocks.iter_mut().find(|node| node.id == *id) else {
                    return Err(EditorError::Transaction(format!(
                        "block '{}' does not exist",
                        id.0
                    )));
                };
                existing.component = component.clone();
            }
            Operation::SetInlineText { block_id, text } => {
                let Some(existing) = document.blocks.iter_mut().find(|node| node.id == *block_id)
                else {
                    return Err(EditorError::Transaction(format!(
                        "block '{}' does not exist",
                        block_id.0
                    )));
                };
                existing.content = NodeContent::Inline(vec![crate::document::InlineNode::text(
                    format!("{}:text", block_id.0),
                    text.clone(),
                )]);
            }
            Operation::SetSelection(next) => {
                *selection = next.clone();
            }
        }
        Ok(())
    }
}

pub fn normalize_document(document: &mut EditorDocument) {
    if document.blocks.is_empty() {
        document
            .blocks
            .push(BlockNode::paragraph("block-1", Vec::new()));
    }
}
