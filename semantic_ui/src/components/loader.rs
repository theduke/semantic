use brass::{
    vdom::{self, TagBuilder},
    VNode,
};

fn spinner() -> TagBuilder {
    vdom::span_with("Loading...")
}

fn error_msg(msg: &str) -> TagBuilder {
    vdom::p_with(msg).class("notification is-danger")
}

pub enum LoadState<T> {
    Idle,
    Loading,
    Success(T),
    Failed(String),
}

impl<T> LoadState<T> {
    pub fn set_idle(&mut self) {
        *self = Self::Idle
    }

    pub fn set_loading(&mut self) {
        *self = Self::Loading
    }

    pub fn set_loaded(&mut self, data: T) {
        *self = Self::Success(data);
    }

    pub fn set_result(&mut self, res: Result<T, impl std::fmt::Display>) {
        match res {
            Ok(data) => *self = Self::Success(data),
            Err(err) => *self = Self::Failed(err.to_string()),
        }
    }

    pub fn set_failed(&mut self, err: impl std::fmt::Display) {
        *self = Self::Failed(err.to_string())
    }

    pub fn render(&self, f: impl Fn(&T) -> VNode) -> VNode {
        match self {
            LoadState::Idle => VNode::Empty,
            LoadState::Loading => spinner().build(),
            LoadState::Success(data) => f(data),
            LoadState::Failed(err) => error_msg(&err).build(),
        }
    }

    /// Returns `true` if the load_state is [`Success`].
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(..))
    }

    /// Returns `true` if the load_state is [`Loading`].
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    pub fn as_success(&self) -> Option<&T> {
        if let Self::Success(v) = self {
            Some(v)
        } else {
            None
        }
    }
}
