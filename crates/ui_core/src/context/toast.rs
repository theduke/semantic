use std::collections::VecDeque;
use std::time::Duration;

use dioxus::prelude::{
    EventHandler, ReadableExt, Signal, WritableExt, use_context, use_context_provider, use_signal,
};

use crate::components::NoticeVariant;

pub(crate) const DEFAULT_MAX_TOASTS: usize = 4;
const INFO_TIMEOUT: Duration = Duration::from_secs(5);
const SUCCESS_TIMEOUT: Duration = Duration::from_secs(4);
const WARNING_TIMEOUT: Duration = Duration::from_secs(8);
const ACTION_TIMEOUT: Duration = Duration::from_secs(8);

/// Stable identifier returned when a toast is dispatched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastId(u64);

impl ToastId {
    /// Numeric value suitable for keyed rendering and diagnostics.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Auto-dismiss policy for a toast.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToastTimeout {
    /// Use the semantic default. Errors remain until dismissed.
    #[default]
    Automatic,
    /// Keep the toast visible until the user dismisses it.
    Persistent,
    /// Dismiss the toast after the supplied duration.
    After(Duration),
}

/// Optional action displayed with a toast.
#[derive(Clone, Debug, PartialEq)]
pub struct ToastAction {
    pub(crate) label: String,
    pub(crate) on_trigger: EventHandler<()>,
}

impl ToastAction {
    /// Create an action. Triggering it also dismisses the owning toast.
    pub fn new(label: impl Into<String>, on_trigger: EventHandler<()>) -> Self {
        Self {
            label: label.into(),
            on_trigger,
        }
    }
}

/// A semantic toast notification.
#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub(crate) title: Option<String>,
    pub(crate) message: String,
    pub(crate) variant: NoticeVariant,
    pub(crate) action: Option<ToastAction>,
    pub(crate) timeout: ToastTimeout,
}

impl Toast {
    pub fn new(message: impl Into<String>, variant: NoticeVariant) -> Self {
        Self {
            title: None,
            message: message.into(),
            variant,
            action: None,
            timeout: ToastTimeout::Automatic,
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(message, NoticeVariant::Info)
    }

    pub fn success(message: impl Into<String>) -> Self {
        Self::new(message, NoticeVariant::Success)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(message, NoticeVariant::Warning)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(message, NoticeVariant::Error)
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn action(mut self, action: ToastAction) -> Self {
        self.action = Some(action);
        self
    }

    pub fn timeout(mut self, timeout: ToastTimeout) -> Self {
        self.timeout = timeout;
        self
    }

    pub(crate) fn resolved_timeout(&self) -> Option<Duration> {
        match self.timeout {
            ToastTimeout::Persistent => None,
            ToastTimeout::After(duration) => Some(duration),
            ToastTimeout::Automatic => match (self.variant, self.action.is_some()) {
                (NoticeVariant::Error, _) => None,
                (_, true) => Some(ACTION_TIMEOUT),
                (NoticeVariant::Info, false) => Some(INFO_TIMEOUT),
                (NoticeVariant::Success, false) => Some(SUCCESS_TIMEOUT),
                (NoticeVariant::Warning, false) => Some(WARNING_TIMEOUT),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToastEntry {
    pub(crate) id: ToastId,
    pub(crate) toast: Toast,
}

#[derive(Debug)]
struct ToastQueue {
    entries: VecDeque<ToastEntry>,
    max_toasts: usize,
    next_id: u64,
}

impl ToastQueue {
    fn new(max_toasts: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_toasts: max_toasts.max(1),
            next_id: 1,
        }
    }

    fn push(&mut self, toast: Toast) -> ToastId {
        let id = self.next_id();
        self.entries.push_back(ToastEntry { id, toast });

        while self.entries.len() > self.max_toasts {
            let previous_entry_count = self.entries.len() - 1;
            let evict = self
                .entries
                .iter()
                .take(previous_entry_count)
                .position(|entry| entry.toast.resolved_timeout().is_some())
                .unwrap_or(0);
            self.entries.remove(evict);
        }

        id
    }

    fn next_id(&mut self) -> ToastId {
        loop {
            let id = ToastId(self.next_id);
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if self.entries.iter().all(|entry| entry.id != id) {
                return id;
            }
        }
    }

    fn dismiss(&mut self, id: ToastId) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.id == id) else {
            return false;
        };
        self.entries.remove(index);
        true
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Copyable dispatcher obtained from [`use_toast_dispatcher`].
///
/// Keeping mutation access here prevents routes from subscribing to the queue.
#[derive(Clone, Copy, PartialEq)]
pub struct ToastDispatcher {
    queue: Signal<ToastQueue>,
}

impl ToastDispatcher {
    /// Add a toast and return its stable ID.
    pub fn show(self, toast: Toast) -> ToastId {
        let mut queue = self.queue;
        queue.write().push(toast)
    }

    /// Dismiss one toast. Returns whether it was still present.
    pub fn dismiss(self, id: ToastId) -> bool {
        let mut queue = self.queue;
        queue.write().dismiss(id)
    }

    /// Dismiss every currently visible toast.
    pub fn clear(self) {
        let mut queue = self.queue;
        queue.write().clear();
    }

    pub(crate) fn entries(self) -> Vec<ToastEntry> {
        self.queue.read().entries.iter().cloned().collect()
    }
}

pub(crate) fn provide_toast_dispatcher(max_toasts: usize) -> ToastDispatcher {
    let queue = use_signal(|| ToastQueue::new(max_toasts));
    use_context_provider(|| ToastDispatcher { queue })
}

/// Access the nearest toast provider without subscribing the calling component.
pub fn use_toast_dispatcher() -> ToastDispatcher {
    use_context::<ToastDispatcher>()
}

#[cfg(test)]
mod tests {
    use super::{Toast, ToastQueue, ToastTimeout};
    use std::time::Duration;

    #[test]
    fn ids_are_stable_and_dismissal_is_targeted() {
        let mut queue = ToastQueue::new(3);
        let first = queue.push(Toast::info("First"));
        let second = queue.push(Toast::success("Second"));

        assert_ne!(first, second);
        assert!(queue.dismiss(first));
        assert!(!queue.dismiss(first));
        assert_eq!(queue.entries.front().map(|entry| entry.id), Some(second));
    }

    #[test]
    fn bounded_queue_evicts_oldest_transient_before_persistent() {
        let mut queue = ToastQueue::new(2);
        let error = queue.push(Toast::error("Could not save"));
        let transient = queue.push(Toast::info("Connected"));
        let newest = queue.push(Toast::success("Saved"));

        let ids = queue
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![error, newest]);
        assert!(!ids.contains(&transient));
    }

    #[test]
    fn persistent_queue_does_not_hide_new_feedback() {
        let mut queue = ToastQueue::new(2);
        let oldest = queue.push(Toast::error("First failure"));
        let retained = queue.push(Toast::error("Second failure"));
        let newest = queue.push(Toast::success("Recovered"));

        let ids = queue
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![retained, newest]);
        assert!(!ids.contains(&oldest));
    }

    #[test]
    fn automatic_timeout_matches_semantics() {
        assert_eq!(
            Toast::success("Saved").resolved_timeout(),
            Some(Duration::from_secs(4))
        );
        assert_eq!(Toast::error("Could not save").resolved_timeout(), None);
    }

    #[test]
    fn explicit_timeout_overrides_error_persistence() {
        let duration = Duration::from_secs(12);
        assert_eq!(
            Toast::error("Retry later")
                .timeout(ToastTimeout::After(duration))
                .resolved_timeout(),
            Some(duration)
        );
        assert_eq!(
            Toast::info("Connection lost")
                .timeout(ToastTimeout::Persistent)
                .resolved_timeout(),
            None
        );
    }
}
