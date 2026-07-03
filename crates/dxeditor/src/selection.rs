use serde::{Deserialize, Serialize};

use crate::document::NodeId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPosition {
    pub block_id: NodeId,
    pub inline_id: Option<NodeId>,
    pub offset: usize,
}

impl TextPosition {
    pub fn new(block_id: impl Into<NodeId>, inline_id: Option<NodeId>, offset: usize) -> Self {
        Self {
            block_id: block_id.into(),
            inline_id,
            offset,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorSelection {
    pub anchor: TextPosition,
    pub focus: TextPosition,
}

impl EditorSelection {
    pub fn collapsed(position: TextPosition) -> Self {
        Self {
            anchor: position.clone(),
            focus: position,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }
}
