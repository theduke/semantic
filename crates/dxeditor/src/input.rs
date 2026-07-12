use serde::{Deserialize, Serialize};

use crate::{
    document::{EditorDocument, NodeId},
    selection::{EditorSelection, TextPosition},
    transaction::{Operation, Transaction},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InputEvent {
    BeforeInput {
        input_type: String,
        #[serde(default)]
        data: String,
        selection: EditorSelection,
    },
    SelectionChange {
        selection: EditorSelection,
    },
    CompositionEnd {
        block_id: NodeId,
        text: String,
        selection: EditorSelection,
    },
    Paste {
        text: String,
        selection: EditorSelection,
    },
}

pub fn transaction_for_event(event: InputEvent, document: &EditorDocument) -> Transaction {
    match event {
        InputEvent::SelectionChange { selection } => selection_transaction(selection),
        InputEvent::Paste { text, selection } => replace_selection(selection, text),
        InputEvent::CompositionEnd {
            block_id,
            text,
            selection,
        } => reconcile_block_text(document, block_id, &text, selection),
        InputEvent::BeforeInput {
            input_type,
            data,
            selection,
        } => match input_type.as_str() {
            "insertText" | "insertReplacementText" => replace_selection(selection, data),
            "insertParagraph" => {
                let offset = selection.focus.offset;
                Transaction::new(vec![Operation::SplitBlock {
                    id: selection.focus.block_id,
                    offset,
                }])
            }
            "deleteContentBackward" => delete(selection, true, document),
            "deleteContentForward" => delete(selection, false, document),
            _ => Transaction::empty(),
        },
    }
}

fn selection_transaction(selection: EditorSelection) -> Transaction {
    Transaction {
        operations: vec![Operation::SetSelection(Some(selection))],
        add_to_history: false,
    }
}

fn replace_selection(selection: EditorSelection, text: String) -> Transaction {
    if selection.anchor.block_id != selection.focus.block_id {
        return Transaction::empty();
    }
    let start = selection.anchor.offset.min(selection.focus.offset);
    let end = selection.anchor.offset.max(selection.focus.offset);
    let block_id = selection.anchor.block_id;
    let mut operations = Vec::new();
    if start != end {
        operations.push(Operation::DeleteText {
            block_id: block_id.clone(),
            range: start..end,
        });
    }
    if !text.is_empty() {
        operations.push(Operation::InsertText {
            block_id: block_id.clone(),
            offset: start,
            text: text.clone(),
        });
    }
    operations.push(Operation::SetSelection(Some(EditorSelection::collapsed(
        TextPosition::new(block_id, None, start + text.chars().count()),
    ))));
    Transaction::new(operations)
}

fn delete(selection: EditorSelection, backward: bool, document: &EditorDocument) -> Transaction {
    if !selection.is_collapsed() {
        return replace_selection(selection, String::new());
    }
    let block_id = selection.focus.block_id.clone();
    let offset = selection.focus.offset;
    if backward && offset == 0 {
        if let Some(index) = document
            .blocks
            .iter()
            .position(|block| block.id == block_id)
            && index > 0
        {
            return Transaction::new(vec![Operation::MergeBlocks {
                first_id: document.blocks[index - 1].id.clone(),
                second_id: block_id,
            }]);
        }
        return Transaction::empty();
    }
    let block_len = document
        .blocks
        .iter()
        .find(|block| block.id == block_id)
        .map(|block| block.text_content().chars().count())
        .unwrap_or_default();
    if !backward && offset >= block_len {
        if let Some(index) = document
            .blocks
            .iter()
            .position(|block| block.id == block_id)
            && let Some(next) = document.blocks.get(index + 1)
        {
            return Transaction::new(vec![Operation::MergeBlocks {
                first_id: block_id,
                second_id: next.id.clone(),
            }]);
        }
        return Transaction::empty();
    }
    let range = if backward {
        offset - 1..offset
    } else {
        offset..offset + 1
    };
    replace_selection(
        EditorSelection {
            anchor: TextPosition::new(block_id.clone(), None, range.start),
            focus: TextPosition::new(block_id, None, range.end),
        },
        String::new(),
    )
}

pub fn reconcile_block_text(
    document: &EditorDocument,
    block_id: NodeId,
    dom_text: &str,
    selection: EditorSelection,
) -> Transaction {
    let model = document
        .blocks
        .iter()
        .find(|block| block.id == block_id)
        .map(|block| block.text_content())
        .unwrap_or_default();
    let old: Vec<char> = model.chars().collect();
    let new: Vec<char> = dom_text.chars().collect();
    let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut operations = Vec::new();
    if prefix + suffix < old.len() {
        operations.push(Operation::DeleteText {
            block_id: block_id.clone(),
            range: prefix..old.len() - suffix,
        });
    }
    if prefix + suffix < new.len() {
        operations.push(Operation::InsertText {
            block_id,
            offset: prefix,
            text: new[prefix..new.len() - suffix].iter().collect(),
        });
    }
    operations.push(Operation::SetSelection(Some(selection)));
    Transaction::new(operations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::BlockNode;

    fn selection(start: usize, end: usize) -> EditorSelection {
        EditorSelection {
            anchor: TextPosition::new("block-1", None, start),
            focus: TextPosition::new("block-1", None, end),
        }
    }

    #[test]
    fn insertion_replaces_the_selected_character_range() {
        let transaction = transaction_for_event(
            InputEvent::BeforeInput {
                input_type: "insertText".into(),
                data: "X".into(),
                selection: selection(1, 3),
            },
            &EditorDocument::plain_text("abcd"),
        );
        assert!(matches!(
            transaction.operations[0],
            Operation::DeleteText {
                range: std::ops::Range { start: 1, end: 3 },
                ..
            }
        ));
        assert!(
            matches!(transaction.operations[1], Operation::InsertText { offset: 1, ref text, .. } if text == "X")
        );
    }

    #[test]
    fn composition_reconcile_emits_a_minimal_middle_edit() {
        let document = EditorDocument::new(vec![BlockNode::paragraph(
            "block-1",
            vec![crate::InlineNode::text("text-1", "abXYcd")],
        )]);
        let transaction =
            reconcile_block_text(&document, "block-1".into(), "abZcd", selection(3, 3));
        assert!(matches!(
            transaction.operations[0],
            Operation::DeleteText {
                range: std::ops::Range { start: 2, end: 4 },
                ..
            }
        ));
        assert!(
            matches!(transaction.operations[1], Operation::InsertText { offset: 2, ref text, .. } if text == "Z")
        );
    }
}
