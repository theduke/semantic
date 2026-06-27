use dioxus::prelude::*;
use dxform::{FormRoot, FormScope};
use semantic_data::value::Value;

use crate::form::SemanticFormMode;

pub fn provide_semantic_form_root(form: FormRoot<Value>) -> FormRoot<Value> {
    use_context_provider(|| form.clone());
    form
}

pub fn use_semantic_form_root() -> FormRoot<Value> {
    use_context()
}

pub fn provide_semantic_form_scope(scope: FormScope<Value, Value>) -> FormScope<Value, Value> {
    use_context_provider(|| scope.clone());
    scope
}

pub fn use_semantic_form_scope() -> FormScope<Value, Value> {
    use_context()
}

pub fn provide_semantic_form_mode(mode: SemanticFormMode) -> SemanticFormMode {
    use_context_provider(|| mode);
    mode
}

pub fn use_semantic_form_mode() -> SemanticFormMode {
    use_context()
}
