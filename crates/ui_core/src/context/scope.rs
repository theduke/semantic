use dioxus::prelude::{
    ReadableExt, Signal, WritableExt, use_context, use_context_provider, use_signal,
};

#[derive(Clone, Copy)]
pub struct UiScopeContext {
    active_scope_id: Signal<Option<String>>,
}

impl UiScopeContext {
    pub fn new(active_scope_id: Signal<Option<String>>) -> Self {
        Self { active_scope_id }
    }

    pub fn active_scope_id(&self) -> Signal<Option<String>> {
        self.active_scope_id
    }

    pub fn set_active_scope_id(&mut self, scope_id: Option<String>) {
        let mut active_scope_id = self.active_scope_id;
        active_scope_id.set(scope_id);
    }
}

pub fn provide_ui_scope_context(scope_id: Option<String>) -> UiScopeContext {
    let active_scope_id = use_signal(|| scope_id);
    use_context_provider(|| UiScopeContext::new(active_scope_id))
}

pub fn use_ui_scope_context() -> UiScopeContext {
    use_context::<UiScopeContext>()
}

pub fn use_active_scope_id() -> Option<String> {
    use_ui_scope_context().active_scope_id.read().clone()
}
