use brass::{
    vdom::{self, TagBuilder},
    VNode,
};

pub fn spinner() -> TagBuilder {
    vdom::span_with("Loading...")
}

pub fn error_msg(msg: &str) -> TagBuilder {
    vdom::p_with(msg).class("notification is-danger")
}

pub enum LoadState<T> {
    Idle,
    Loading(Option<brass::EffectGuard>),
    Success(T),
    Failed(String),
}

impl<T> LoadState<T> {
    pub fn set_idle(&mut self) {
        *self = Self::Idle
    }

    pub fn set_loading(&mut self) {
        *self = Self::Loading(None)
    }

    pub fn set_loading_guarded(&mut self, guard: brass::EffectGuard) {
        *self = Self::Loading(Some(guard))
    }

    pub fn set_success(&mut self, data: T) {
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

    pub fn render(&self, f: impl FnOnce(&T) -> VNode) -> VNode {
        match self {
            LoadState::Idle => VNode::Empty,
            LoadState::Loading(_) => spinner().build(),
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
        matches!(self, Self::Loading(_))
    }

    /// Returns `true` if the load_state is [`Failed`].
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    pub fn as_success(&self) -> Option<&T> {
        if let Self::Success(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn as_error(&self) -> Option<&str> {
        if let Self::Failed(err) = self {
            Some(err.as_str())
        } else {
            None
        }
    }
}

impl<T, E: std::fmt::Display> From<Result<T, E>> for LoadState<T> {
    fn from(res: Result<T, E>) -> Self {
        match res {
            Ok(data) => LoadState::Success(data),
            Err(err) => LoadState::Failed(err.to_string()),
        }
    }
}
