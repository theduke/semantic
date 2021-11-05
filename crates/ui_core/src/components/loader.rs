use std::fmt::Display;

use brass::{
    dom::{
        builder::{div, p, span},
        Attr, TagBuilder,
    },
    effect::{spawn_guarded, EffectGuard},
    signal::signal::{Mutable, Signal},
};
use factordb::AnyError;
use futures::Future;

pub fn spinner() -> TagBuilder {
    div().and("Loading...")
}

pub fn error_msg(msg: &str) -> TagBuilder {
    p().and(msg).attr(Attr::Class, "notification is-danger")
}

pub enum LoadState<T> {
    Idle,
    Loading(Option<EffectGuard>),
    Success(T),
    Failed(String),
}

impl<T: Clone> Clone for LoadState<T> {
    /// Clone, but ignore a potential `[EffectGuard]` inside [`Self::Loading`].
    fn clone(&self) -> Self {
        match self {
            Self::Idle => Self::Idle,
            Self::Loading(_) => Self::Loading(None),
            Self::Success(arg0) => Self::Success(arg0.clone()),
            Self::Failed(arg0) => Self::Failed(arg0.clone()),
        }
    }
}

impl<T> LoadState<T> {
    pub fn from_res(res: Result<T, impl Display>) -> Self {
        match res {
            Ok(v) => Self::Success(v),
            Err(err) => Self::Failed(err.to_string()),
        }
    }

    pub fn set_idle(&mut self) {
        *self = Self::Idle
    }

    pub fn set_loading(&mut self) {
        *self = Self::Loading(None)
    }

    pub fn set_loading_guarded(&mut self, guard: EffectGuard) {
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

    pub fn render(&self, f: impl FnOnce(&T) -> TagBuilder) -> TagBuilder {
        match self {
            LoadState::Idle => span(),
            // TODO: restore DelayedSpinner
            // LoadState::Loading(_) => crate::components::DelayedSpinner {}.render(),
            LoadState::Loading(_) => spinner(),
            LoadState::Success(data) => f(data),
            LoadState::Failed(err) => error_msg(&err),
        }
    }

    fn render_idle(
        &self,
        render_idle: impl FnOnce() -> TagBuilder,
        render_success: impl FnOnce(&T) -> TagBuilder,
    ) -> TagBuilder {
        match self {
            LoadState::Idle => render_idle(),
            LoadState::Loading(_) => spinner(),
            LoadState::Success(data) => render_success(data),
            LoadState::Failed(err) => error_msg(&err),
        }
    }

    pub fn render_no_content(&self) -> TagBuilder {
        match self {
            Self::Idle | Self::Success(_) => span(),
            // TODO: restore DelayedSpinner
            // Self::Loading(_) => crate::components::DelayedSpinner {}.render(),
            Self::Loading(_) => spinner(),
            Self::Failed(err) => error_msg(&err),
        }
    }

    /// Returns `true` if the load state is [`Idle`].
    ///
    /// [`Idle`]: LoadState::Idle
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
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

    pub fn as_success_mut(&mut self) -> Option<&mut T> {
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

impl LoadState<()> {
    pub fn set_result_take<T>(&mut self, res: Result<T, impl std::fmt::Display>) -> Option<T> {
        match res {
            Ok(data) => {
                *self = Self::Success(());
                Some(data)
            }
            Err(err) => {
                *self = Self::Failed(err.to_string());
                None
            }
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

pub struct Loader<T>(Mutable<LoadState<T>>);

impl<T> Clone for Loader<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Loader<T> {
    pub fn new_idle() -> Self {
        Self(Mutable::new(LoadState::Idle))
    }

    pub fn is_loading(&self) -> bool {
        self.0.lock_ref().is_loading()
    }

    pub fn is_idle(&self) -> bool {
        self.0.lock_ref().is_idle()
    }

    pub fn get_cloned(&self) -> LoadState<T>
    where
        T: Clone,
    {
        self.0.lock_ref().clone()
    }

    pub fn new_spawn(f: impl Future<Output = Result<T, AnyError>> + 'static) -> Self
    where
        T: 'static,
    {
        let mutable = Mutable::new(LoadState::Loading(None));
        let mutable2 = mutable.clone();
        let guard = spawn_guarded(async move {
            let res = f.await;
            mutable2.set(LoadState::from_res(res));
        });
        mutable.set(LoadState::Loading(Some(guard)));
        Self(mutable)
    }

    pub fn get(&self) -> &Mutable<LoadState<T>> {
        &self.0
    }

    pub fn spawn(&self, f: impl Future<Output = Result<T, AnyError>> + 'static)
    where
        T: 'static,
    {
        let handle = self.0.clone();
        let guard = spawn_guarded(async move {
            let res = f.await;
            handle.set(LoadState::from_res(res));
        });
        self.0.set(LoadState::Loading(Some(guard)));
    }

    pub fn set_loading(&mut self, guard: EffectGuard) {
        self.0.set(LoadState::Loading(Some(guard)));
    }

    pub fn set_idle(&mut self) {
        self.0.set(LoadState::Idle);
    }

    pub fn set_result(&mut self, res: Result<T, AnyError>) {
        self.0.set(LoadState::from_res(res));
    }

    pub fn signal_loading(&self) -> impl Signal<Item = bool> + 'static
    where
        T: 'static,
    {
        self.0.signal_ref(|s| s.is_loading())
    }

    // FIXME: this method makes it easy to accidentally drop the loader, which
    // can cause rendering issues.
    // Find a better API design.
    pub fn signal_render(
        &self,
        render: impl Fn(&T) -> TagBuilder,
    ) -> impl Signal<Item = TagBuilder> {
        self.0.signal_ref(move |state| state.render(&render))
    }

    // FIXME: this method makes it easy to accidentally drop the loader, which
    // can cause rendering issues.
    // Find a better API design.
    pub fn signal_render_state(
        &self,
        render: impl Fn(&LoadState<T>) -> TagBuilder,
    ) -> impl Signal<Item = TagBuilder> {
        self.0.signal_ref(move |state| render(state))
    }

    // FIXME: this method makes it easy to accidentally drop the loader, which
    // can cause rendering issues.
    // Find a better API design.
    pub fn signal_render_idle(
        &self,
        render_idle: impl Fn() -> TagBuilder,
        render_success: impl Fn(&T) -> TagBuilder,
    ) -> impl Signal<Item = TagBuilder> {
        self.0
            .signal_ref(move |state| state.render_idle(&render_idle, &render_success))
    }
}

impl Loader<()> {
    pub fn set_result_take<T>(&self, res: Result<T, impl std::fmt::Display>) -> Option<T> {
        self.0.lock_mut().set_result_take(res)
    }
}

pub fn load<T: 'static>(
    f: impl Future<Output = Result<T, AnyError>> + 'static,
    render: impl Fn(&T) -> TagBuilder + 'static,
) -> TagBuilder {
    let loader = Loader::new_spawn(f);
    div()
        .child_signal(loader.signal_render(render))
        .bind(loader)
}
