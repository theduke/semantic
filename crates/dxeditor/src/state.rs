use std::{cell::RefCell, rc::Rc};

use crate::{
    EditorError, document::EditorDocument, selection::EditorSelection, transaction::Transaction,
};

#[derive(Clone, Debug, PartialEq)]
pub struct EditorHistory {
    undo: Vec<EditorDocument>,
    redo: Vec<EditorDocument>,
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
        let previous = data.document.clone();
        let mut next_document = data.document.clone();
        let mut next_selection = data.selection.clone();
        transaction.apply(&mut next_document, &mut next_selection)?;
        data.document = next_document;
        data.selection = next_selection;

        if transaction.add_to_history {
            data.history.undo.push(previous);
            data.history.redo.clear();
        }
        Ok(())
    }

    pub fn undo(&self) -> bool {
        let mut data = self.inner.borrow_mut();
        let Some(previous) = data.history.undo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut data.document, previous);
        data.history.redo.push(current);
        true
    }

    pub fn redo(&self) -> bool {
        let mut data = self.inner.borrow_mut();
        let Some(next) = data.history.redo.pop() else {
            return false;
        };
        let current = std::mem::replace(&mut data.document, next);
        data.history.undo.push(current);
        true
    }
}

impl PartialEq for EditorState {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner) || *self.inner.borrow() == *other.inner.borrow()
    }
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
