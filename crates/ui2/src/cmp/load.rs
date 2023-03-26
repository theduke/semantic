use dioxus::prelude::*;

use crate::cmp::bootstrap::Alert;

pub fn render_future_res<'a, T, E: std::fmt::Display>(
    cx: Scope<'a>,
    fut: &'a UseFuture<Result<T, E>>,
    render: impl Fn(&'a T) -> Element<'a>,
) -> Element<'a> {
    match fut.value() {
        Some(Ok(value)) => render(value),
        Some(Err(err)) => cx.render(rsx!(
            Alert {
                color: crate::cmp::bootstrap::Color::Danger,
                "{err}"
            }
        )),
        None => cx.render(rsx!(div {
            "Loading..."
        })),
    }
}

pub enum Loader<T, E: std::fmt::Display = String> {
    Idle,
    Loading,
    Ready(T),
    Error(E),
}

impl<T, E: std::fmt::Display> Loader<T, E> {
    pub fn from_result(result: Result<T, E>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(err) => Self::Error(err),
        }
    }

    /// Returns `true` if the loader is [`Loading`].
    ///
    /// [`Loading`]: Loader::Loading
    #[must_use]
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// Returns `true` if the loader is [`Ready`].
    ///
    /// [`Ready`]: Loader::Ready
    #[must_use]
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready(..))
    }

    /// Returns `true` if the loader is [`Error`].
    ///
    /// [`Error`]: Loader::Error
    #[must_use]
    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(..))
    }

    pub fn set_loading(&mut self) {
        *self = Self::Loading;
    }

    pub fn set_result(&mut self, result: Result<T, E>) {
        *self = match result {
            Ok(value) => Self::Ready(value),
            Err(err) => Self::Error(err),
        };
    }

    pub fn render<'a>(
        &'a self,
        cx: Scope<'a>,
        render: impl FnOnce(Scope<'a>, &'a T) -> Element<'a>,
    ) -> Element {
        match self {
            Self::Idle => Element::default(),
            Self::Loading => cx.render(rsx!(div {
                class: "spinner-grow",
                role: "status",
                span {
                    class: "visually-hidden",
                    "Loading..."
                }
            })),
            Self::Error(err) => cx.render(rsx!(
                    Alert {
                        color: crate::cmp::bootstrap::Color::Danger,
                        "{err}"
                    }
            )),
            Self::Ready(value) => render(cx, value),
        }
    }
}

impl<T, E: std::fmt::Display> Default for Loader<T, E> {
    fn default() -> Self {
        Self::Idle
    }
}

impl<T, E: std::fmt::Display> From<Result<T, E>> for Loader<T, E> {
    fn from(result: Result<T, E>) -> Self {
        Self::from_result(result)
    }
}
