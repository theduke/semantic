use crate::{
    EditorError,
    document::{BlockNode, EditorDocument, InlineNode, Mark, NodeContent, NodeId},
    selection::{EditorSelection, TextPosition},
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
    InsertBlock {
        index: usize,
        block: BlockNode,
    },
    ReplaceBlock {
        id: NodeId,
        block: BlockNode,
    },
    RemoveBlock {
        id: NodeId,
    },
    SetBlockComponent {
        id: NodeId,
        component: String,
    },
    SetBlockType {
        id: NodeId,
        component: String,
        attrs: serde_json::Map<String, serde_json::Value>,
    },
    SetInlineText {
        block_id: NodeId,
        text: String,
    },
    SetInlineContent {
        block_id: NodeId,
        inline: Vec<InlineNode>,
    },
    ToggleMark {
        selection: EditorSelection,
        mark: Mark,
    },
    SplitBlock {
        id: NodeId,
        offset: usize,
    },
    MergeBlocks {
        first_id: NodeId,
        second_id: NodeId,
    },
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
            Operation::SetBlockType {
                id,
                component,
                attrs,
            } => {
                let Some(existing) = document.blocks.iter_mut().find(|node| node.id == *id) else {
                    return Err(EditorError::Transaction(format!(
                        "block '{}' does not exist",
                        id.0
                    )));
                };
                existing.component = component.clone();
                existing.attrs = attrs.clone();
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
            Operation::SetInlineContent { block_id, inline } => {
                let Some(existing) = document.blocks.iter_mut().find(|node| node.id == *block_id)
                else {
                    return Err(EditorError::Transaction(format!(
                        "block '{}' does not exist",
                        block_id.0
                    )));
                };
                existing.content = NodeContent::Inline(inline.clone());
            }
            Operation::ToggleMark { selection, mark } => {
                toggle_mark(document, selection, mark)?;
            }
            Operation::SplitBlock { id, offset } => {
                split_block(document, id, *offset, selection)?;
            }
            Operation::MergeBlocks {
                first_id,
                second_id,
            } => {
                merge_blocks(document, first_id, second_id, selection)?;
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

fn inline_content_mut<'a>(
    document: &'a mut EditorDocument,
    block_id: &NodeId,
) -> Result<&'a mut Vec<InlineNode>, EditorError> {
    let Some(block) = document
        .blocks
        .iter_mut()
        .find(|block| block.id == *block_id)
    else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not exist",
            block_id.0
        )));
    };
    let NodeContent::Inline(inline) = &mut block.content else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not contain inline content",
            block_id.0
        )));
    };
    Ok(inline)
}

fn inline_content<'a>(
    document: &'a EditorDocument,
    block_id: &NodeId,
) -> Result<&'a Vec<InlineNode>, EditorError> {
    let Some(block) = document.blocks.iter().find(|block| block.id == *block_id) else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not exist",
            block_id.0
        )));
    };
    let NodeContent::Inline(inline) = &block.content else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not contain inline content",
            block_id.0
        )));
    };
    Ok(inline)
}

fn toggle_mark(
    document: &mut EditorDocument,
    selection: &EditorSelection,
    mark: &Mark,
) -> Result<(), EditorError> {
    if selection.anchor.block_id != selection.focus.block_id || selection.is_collapsed() {
        return Ok(());
    }

    let block_id = selection.anchor.block_id.clone();
    let inline = inline_content(document, &block_id)?;
    let anchor = absolute_offset(inline, &selection.anchor)?;
    let focus = absolute_offset(inline, &selection.focus)?;
    let start = anchor.min(focus);
    let end = anchor.max(focus);
    if start == end {
        return Ok(());
    }

    let inline = inline_content_mut(document, &block_id)?;
    let all_marked = inline
        .iter()
        .filter(|node| node_range_intersects(node, inline, start, end))
        .all(|node| has_equivalent_mark(node, mark));
    let mut cursor = 0usize;
    let mut next = Vec::new();

    for node in inline.iter() {
        let node_len = node.text.chars().count();
        let node_start = cursor;
        let node_end = cursor + node_len;
        cursor = node_end;

        if node_end <= start || node_start >= end || node_len == 0 {
            next.push(node.clone());
            continue;
        }

        let local_start = start.saturating_sub(node_start).min(node_len);
        let local_end = end.saturating_sub(node_start).min(node_len);
        push_mark_segment(&mut next, node, 0, local_start, None);
        push_mark_segment(
            &mut next,
            node,
            local_start,
            local_end,
            Some((mark, all_marked)),
        );
        push_mark_segment(&mut next, node, local_end, node_len, None);
    }

    *inline = compact_inline(next);
    Ok(())
}

fn node_range_intersects(
    node: &InlineNode,
    inline: &[InlineNode],
    start: usize,
    end: usize,
) -> bool {
    let mut cursor = 0usize;
    for candidate in inline {
        let node_start = cursor;
        let node_end = cursor + candidate.text.chars().count();
        cursor = node_end;
        if candidate.id == node.id {
            return node_end > start && node_start < end;
        }
    }
    false
}

fn push_mark_segment(
    output: &mut Vec<InlineNode>,
    node: &InlineNode,
    start: usize,
    end: usize,
    mark_action: Option<(&Mark, bool)>,
) {
    if start == end {
        return;
    }
    let mut segment = node.clone();
    segment.id = NodeId::from(format!("{}:{}-{end}", node.id.0, start));
    segment.text = slice_chars(&node.text, start, end);
    if let Some((mark, remove)) = mark_action {
        if remove {
            segment
                .marks
                .retain(|existing| !same_mark_kind(existing, mark));
        } else if !has_equivalent_mark(&segment, mark) {
            segment.marks.push(mark.clone());
        }
    }
    output.push(segment);
}

fn split_block(
    document: &mut EditorDocument,
    block_id: &NodeId,
    offset: usize,
    selection: &mut Option<EditorSelection>,
) -> Result<(), EditorError> {
    let Some(index) = document
        .blocks
        .iter()
        .position(|block| block.id == *block_id)
    else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not exist",
            block_id.0
        )));
    };
    let mut block = document.blocks[index].clone();
    let NodeContent::Inline(inline) = &block.content else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not contain inline content",
            block_id.0
        )));
    };
    let (left, right) = split_inline_at(inline, offset);
    let next_id = unique_block_id(document, &format!("{}-split", block_id.0));
    block.content = NodeContent::Inline(left);
    document.blocks[index] = block;

    let mut right_block = document.blocks[index].clone();
    right_block.id = next_id.clone();
    right_block.content = NodeContent::Inline(right);
    document.blocks.insert(index + 1, right_block);
    *selection = Some(EditorSelection::collapsed(TextPosition::new(
        next_id, None, 0,
    )));
    Ok(())
}

fn merge_blocks(
    document: &mut EditorDocument,
    first_id: &NodeId,
    second_id: &NodeId,
    selection: &mut Option<EditorSelection>,
) -> Result<(), EditorError> {
    let Some(first_index) = document
        .blocks
        .iter()
        .position(|block| block.id == *first_id)
    else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not exist",
            first_id.0
        )));
    };
    if document
        .blocks
        .get(first_index + 1)
        .is_none_or(|block| block.id != *second_id)
    {
        return Err(EditorError::Transaction(format!(
            "blocks '{}' and '{}' are not adjacent",
            first_id.0, second_id.0
        )));
    }

    let second = document.blocks.remove(first_index + 1);
    let NodeContent::Inline(second_inline) = second.content else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not contain inline content",
            second_id.0
        )));
    };
    let first = &mut document.blocks[first_index];
    let NodeContent::Inline(first_inline) = &mut first.content else {
        return Err(EditorError::Transaction(format!(
            "block '{}' does not contain inline content",
            first_id.0
        )));
    };
    let offset = first_inline
        .iter()
        .map(|node| node.text.chars().count())
        .sum();
    first_inline.extend(second_inline);
    *first_inline = compact_inline(std::mem::take(first_inline));
    *selection = Some(EditorSelection::collapsed(TextPosition::new(
        first_id.clone(),
        None,
        offset,
    )));
    Ok(())
}

fn split_inline_at(inline: &[InlineNode], offset: usize) -> (Vec<InlineNode>, Vec<InlineNode>) {
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut cursor = 0usize;

    for node in inline {
        let len = node.text.chars().count();
        let end = cursor + len;
        if end <= offset {
            left.push(node.clone());
        } else if cursor >= offset {
            right.push(node.clone());
        } else {
            let split_at = offset - cursor;
            let mut left_node = node.clone();
            left_node.id = NodeId::from(format!("{}:left", node.id.0));
            left_node.text = slice_chars(&node.text, 0, split_at);
            let mut right_node = node.clone();
            right_node.id = NodeId::from(format!("{}:right", node.id.0));
            right_node.text = slice_chars(&node.text, split_at, len);
            if !left_node.text.is_empty() {
                left.push(left_node);
            }
            if !right_node.text.is_empty() {
                right.push(right_node);
            }
        }
        cursor = end;
    }

    (compact_inline(left), compact_inline(right))
}

fn absolute_offset(inline: &[InlineNode], position: &TextPosition) -> Result<usize, EditorError> {
    if let Some(inline_id) = &position.inline_id {
        let mut cursor = 0usize;
        for node in inline {
            if node.id == *inline_id {
                return Ok(cursor + position.offset.min(node.text.chars().count()));
            }
            cursor += node.text.chars().count();
        }
        return Err(EditorError::Transaction(format!(
            "inline node '{}' does not exist",
            inline_id.0
        )));
    }

    let total = inline.iter().map(|node| node.text.chars().count()).sum();
    Ok(position.offset.min(total))
}

fn compact_inline(inline: Vec<InlineNode>) -> Vec<InlineNode> {
    let mut output: Vec<InlineNode> = Vec::new();
    for node in inline.into_iter().filter(|node| !node.text.is_empty()) {
        if let Some(previous) = output.last_mut()
            && previous.component == node.component
            && previous.attrs == node.attrs
            && previous.marks == node.marks
        {
            previous.text.push_str(&node.text);
            continue;
        }
        output.push(node);
    }
    output
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars().skip(start).take(end - start).collect()
}

fn has_equivalent_mark(node: &InlineNode, mark: &Mark) -> bool {
    node.marks
        .iter()
        .any(|existing| same_mark_kind(existing, mark) && existing.attrs == mark.attrs)
}

fn same_mark_kind(left: &Mark, right: &Mark) -> bool {
    left.component == right.component
}

fn unique_block_id(document: &EditorDocument, seed: &str) -> NodeId {
    let mut candidate = seed.to_string();
    let mut index = 1usize;
    while document.blocks.iter().any(|block| block.id.0 == candidate) {
        index += 1;
        candidate = format!("{seed}-{index}");
    }
    NodeId::from(candidate)
}
