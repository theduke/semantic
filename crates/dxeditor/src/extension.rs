use crate::{EditorError, catalog::EditorCatalog};

pub trait EditorExtension {
    fn register(&self, catalog: &mut EditorCatalog) -> Result<(), EditorError>;
}
