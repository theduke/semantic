use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use crate::{
    EditorError, document::EditorDocument, selection::EditorSelection, transaction::Transaction,
};

#[derive(Clone, Debug, PartialEq)]
pub struct EditorHistory {
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
}

const HISTORY_LIMIT: usize = 200;
const COALESCE_WINDOW: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq)]
struct HistoryEntry {
    inverse: Transaction,
    selection_before: Option<EditorSelection>,
    inserted: Option<(crate::document::NodeId, usize, usize)>,
    timestamp: Instant,
}

impl EditorHistory {
    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }
}

impl Default for EditorHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EditorStateData {
    document: EditorDocument,
    selection: Option<EditorSelection>,
    history: EditorHistory,
    readonly: bool,
}

#[derive(Clone, Debug)]
pub struct EditorState {
    inner: Rc<RefCell<EditorStateData>>,
}

impl EditorState {
    pub fn new(document: EditorDocument) -> Self {
        Self {
            inner: Rc::new(RefCell::new(EditorStateData {
                document,
                selection: None,
                history: EditorHistory::new(),
                readonly: false,
            })),
        }
    }

    pub fn readonly(document: EditorDocument) -> Self {
        let state = Self::new(document);
        state.set_readonly(true);
        state
    }

    pub fn document(&self) -> EditorDocument {
        self.inner.borrow().document.clone()
    }

    pub fn selection(&self) -> Option<EditorSelection> {
        self.inner.borrow().selection.clone()
    }

    pub fn history(&self) -> EditorHistory {
        self.inner.borrow().history.clone()
    }

    pub fn readonly_enabled(&self) -> bool {
        self.inner.borrow().readonly
    }

    pub fn set_readonly(&self, readonly: bool) {
        self.inner.borrow_mut().readonly = readonly;
    }

    pub fn apply_transaction(&self, transaction: Transaction) -> Result<(), EditorError> {
        if self.readonly_enabled() {
            return Ok(());
        }

        let mut data = self.inner.borrow_mut();
        let mut next_document = data.document.clone();
        let mut next_selection = data.selection.clone();
        let selection_before = next_selection.clone();
        let inverse = transaction.apply_with_inverse(&mut next_document, &mut next_selection)?;
        data.document = next_document;
        data.selection = next_selection;

        if transaction.add_to_history {
            let now = Instant::now();
            let inserted = inserted_range(&transaction);
            let should_coalesce = data.history.undo.last().is_some_and(|previous| {
                matches!((&previous.inserted, &inserted),
                    (Some((old_block, _, old_end)), Some((new_block, new_start, _)))
                        if old_block == new_block && old_end == new_start)
                    && now.duration_since(previous.timestamp) <= COALESCE_WINDOW
            });
            if should_coalesce {
                let previous = data.history.undo.last_mut().expect("checked above");
                let mut operations = inverse.operations;
                operations.extend(std::mem::take(&mut previous.inverse.operations));
                previous.inverse.operations = operations;
                if let (Some((_, _, old_end)), Some((_, _, new_end))) =
                    (&mut previous.inserted, inserted)
                {
                    *old_end = new_end;
                }
                previous.timestamp = now;
            } else {
                data.history.undo.push(HistoryEntry {
                    inverse,
                    selection_before,
                    inserted,
                    timestamp: now,
                });
                if data.history.undo.len() > HISTORY_LIMIT {
                    data.history.undo.remove(0);
                }
            }
            data.history.redo.clear();
        }
        Ok(())
    }

    pub fn undo(&self) -> bool {
        let mut data = self.inner.borrow_mut();
        let Some(previous) = data.history.undo.pop() else {
            return false;
        };
        let selection_before = data.selection.clone();
        let mut document = data.document.clone();
        let mut selection = data.selection.clone();
        let Ok(inverse) = previous
            .inverse
            .apply_with_inverse(&mut document, &mut selection)
        else {
            data.history.undo.push(previous);
            return false;
        };
        data.document = document;
        data.selection = previous.selection_before;
        data.history.redo.push(HistoryEntry {
            inverse,
            selection_before,
            inserted: None,
            timestamp: Instant::now(),
        });
        true
    }

    pub fn redo(&self) -> bool {
        let mut data = self.inner.borrow_mut();
        let Some(next) = data.history.redo.pop() else {
            return false;
        };
        let selection_before = data.selection.clone();
        let mut document = data.document.clone();
        let mut selection = data.selection.clone();
        let Ok(inverse) = next
            .inverse
            .apply_with_inverse(&mut document, &mut selection)
        else {
            data.history.redo.push(next);
            return false;
        };
        data.document = document;
        data.selection = next.selection_before;
        data.history.undo.push(HistoryEntry {
            inverse,
            selection_before,
            inserted: None,
            timestamp: Instant::now(),
        });
        true
    }
}

impl PartialEq for EditorState {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

fn inserted_range(transaction: &Transaction) -> Option<(crate::document::NodeId, usize, usize)> {
    let [
        crate::transaction::Operation::InsertText {
            block_id,
            offset,
            text,
        },
    ] = transaction.operations.as_slice()
    else {
        return None;
    };
    Some((block_id.clone(), *offset, offset + text.chars().count()))
}

#[derive(Clone)]
pub struct EditorHandle {
    state: EditorState,
}

impl EditorHandle {
    pub fn new(state: EditorState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> EditorState {
        self.state.clone()
    }

    pub fn apply_transaction(&self, transaction: Transaction) -> Result<(), EditorError> {
        self.state.apply_transaction(transaction)
    }
}
