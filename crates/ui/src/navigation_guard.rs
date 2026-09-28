//! Blocks navigation away from pages with unsaved changes.
//!
//! The Dioxus router has no navigation-blocking hook, so [`GuardedHistory`]
//! wraps the platform history provider and holds back router navigation
//! (links, `navigator().push`, back/forward) and browser history traversal
//! while the registered page is dirty. [`NavigationGuardPrompt`] then asks the
//! user to save, discard, or keep editing. Leaving the web app entirely (tab
//! close, reload, external links) is covered by a `beforeunload` handler.

use std::{rc::Rc, sync::Arc};

use dioxus::{
    history::{History, history, provide_history_context},
    prelude::*,
};

/// A router navigation that was held back because the page had unsaved changes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingNavigation {
    Push(String),
    Replace(String),
    Back,
    Forward,
    /// The browser already changed the URL (back/forward buttons); the guarded
    /// route must be restored before the user decides.
    BrowserTraversal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuardEntry {
    id: u64,
    /// Route of the guarded page, used to undo browser history traversal.
    route: String,
    dirty: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct GuardState {
    entry: Option<GuardEntry>,
    pending: Option<PendingNavigation>,
    /// A save was requested from the prompt; `pending` resumes once it succeeds.
    saving: bool,
    next_id: u64,
}

impl GuardState {
    fn is_blocking(&self) -> bool {
        self.entry.as_ref().is_some_and(|entry| entry.dirty)
    }

    fn register(&mut self, route: String) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.entry = Some(GuardEntry {
            id,
            route,
            dirty: false,
        });
        self.pending = None;
        self.saving = false;
        id
    }

    fn unregister(&mut self, id: u64) {
        if self.entry.as_ref().is_some_and(|entry| entry.id == id) {
            self.entry = None;
            self.pending = None;
            self.saving = false;
        }
    }

    fn set_dirty(&mut self, id: u64, dirty: bool) {
        if let Some(entry) = self.entry.as_mut().filter(|entry| entry.id == id) {
            entry.dirty = dirty;
        }
    }

    /// Records `navigation` as pending and returns `true` if it must be blocked.
    fn intercept(&mut self, navigation: PendingNavigation) -> bool {
        if !self.is_blocking() {
            return false;
        }
        self.pending = Some(navigation);
        true
    }

    /// Stops guarding the current page and returns the navigation to perform.
    fn release(&mut self) -> Option<PendingNavigation> {
        if let Some(entry) = self.entry.as_mut() {
            entry.dirty = false;
        }
        self.saving = false;
        self.pending.take()
    }

    fn prompt_open(&self) -> bool {
        !self.saving
            && self
                .pending
                .as_ref()
                .is_some_and(|pending| *pending != PendingNavigation::BrowserTraversal)
    }
}

/// App-wide navigation guard state, provided by [`use_navigation_guard_provider`].
#[derive(Clone, Copy)]
struct NavigationGuard {
    /// Sync storage because the browser traversal callback must be `Send + Sync`.
    state: Signal<GuardState, SyncStorage>,
    save: Signal<Option<(u64, Callback<()>)>>,
    inner_history: CopyValue<Rc<dyn History>>,
}

impl NavigationGuard {
    fn inner_history(&self) -> Rc<dyn History> {
        self.inner_history.read().clone()
    }

    /// Puts a browser-changed URL back to the guarded route and turns the
    /// traversal into an ordinary pending push to the requested route.
    fn restore_after_traversal(mut self) {
        let history = self.inner_history();
        let target = history.current_route();
        let mut state = self.state.write();
        let Some(route) = state.entry.as_ref().map(|entry| entry.route.clone()) else {
            state.pending = None;
            return;
        };
        if target == route {
            state.pending = None;
            return;
        }
        history.push(route);
        state.pending = Some(PendingNavigation::Push(target));
    }

    fn discard(mut self) {
        let pending = self.state.write().release();
        if let Some(pending) = pending {
            perform(pending);
        }
    }

    fn keep_editing(mut self) {
        let mut state = self.state.write();
        state.pending = None;
        state.saving = false;
    }

    fn save_handler(&self) -> Option<Callback<()>> {
        let state = self.state.read();
        let entry_id = state.entry.as_ref()?.id;
        self.save
            .read()
            .as_ref()
            .filter(|(id, _)| *id == entry_id)
            .map(|(_, save)| *save)
    }

    fn save(mut self) {
        let Some(save) = self.save_handler() else {
            return;
        };
        self.state.write().saving = true;
        save.call(());
    }
}

fn perform(navigation: PendingNavigation) {
    let router = router();
    match navigation {
        PendingNavigation::Push(route) => {
            router.push(NavigationTarget::<String>::Internal(route));
        }
        PendingNavigation::Replace(route) => {
            router.replace(NavigationTarget::<String>::Internal(route));
        }
        PendingNavigation::Back => router.go_back(),
        PendingNavigation::Forward => router.go_forward(),
        PendingNavigation::BrowserTraversal => {}
    }
}

/// Wraps the platform history so navigation can be held back while guarded.
struct GuardedHistory {
    inner: Rc<dyn History>,
    state: Signal<GuardState, SyncStorage>,
}

impl GuardedHistory {
    fn intercept(&self, navigation: PendingNavigation) -> bool {
        let mut state = self.state;
        state.with_mut(|state| state.intercept(navigation))
    }
}

impl History for GuardedHistory {
    fn current_route(&self) -> String {
        self.inner.current_route()
    }

    fn current_prefix(&self) -> Option<String> {
        self.inner.current_prefix()
    }

    fn can_go_back(&self) -> bool {
        self.inner.can_go_back()
    }

    fn go_back(&self) {
        if !self.intercept(PendingNavigation::Back) {
            self.inner.go_back();
        }
    }

    fn can_go_forward(&self) -> bool {
        self.inner.can_go_forward()
    }

    fn go_forward(&self) {
        if !self.intercept(PendingNavigation::Forward) {
            self.inner.go_forward();
        }
    }

    fn push(&self, route: String) {
        if !self.intercept(PendingNavigation::Push(route.clone())) {
            self.inner.push(route);
        }
    }

    fn replace(&self, path: String) {
        if !self.intercept(PendingNavigation::Replace(path.clone())) {
            self.inner.replace(path);
        }
    }

    fn external(&self, url: String) -> bool {
        self.inner.external(url)
    }

    fn updater(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let state = self.state;
        self.inner.updater(Arc::new(move || {
            // The browser has already changed the URL; only notify the router
            // when the page is not guarded. Otherwise the prompt restores it.
            let mut state = state;
            if !state.with_mut(|state| state.intercept(PendingNavigation::BrowserTraversal)) {
                callback();
            }
        }));
    }

    fn include_prevent_default(&self) -> bool {
        self.inner.include_prevent_default()
    }
}

/// Installs the guarded history provider. Must be called above the router.
pub(crate) fn use_navigation_guard_provider() {
    use_hook(|| {
        let inner = history();
        let state = Signal::new_maybe_sync(GuardState::default());
        let guard = NavigationGuard {
            state,
            save: Signal::new(None),
            inner_history: CopyValue::new(inner.clone()),
        };
        provide_context(guard);
        provide_history_context(Rc::new(GuardedHistory { inner, state }));
        install_unload_guard(state);
    });
}

#[cfg(target_arch = "wasm32")]
fn install_unload_guard(state: Signal<GuardState, SyncStorage>) {
    use wasm_bindgen::{JsCast, prelude::Closure};

    let Some(window) = web_sys::window() else {
        return;
    };
    let listener = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        if state.peek().is_blocking() {
            event.prevent_default();
            // Legacy browsers only show the prompt when `returnValue` is set.
            let _ = js_sys::Reflect::set(&event, &"returnValue".into(), &"".into());
        }
    });
    let _ =
        window.add_event_listener_with_callback("beforeunload", listener.as_ref().unchecked_ref());
    // The app root lives for the whole page lifetime.
    listener.forget();
}

#[cfg(not(target_arch = "wasm32"))]
fn install_unload_guard(_state: Signal<GuardState, SyncStorage>) {}

/// Handle for a page that guards its unsaved changes against navigation.
#[derive(Clone, Copy)]
pub(crate) struct PageNavigationGuard {
    guard: NavigationGuard,
    id: u64,
}

impl PageNavigationGuard {
    pub(crate) fn set_dirty(mut self, dirty: bool) {
        let id = self.id;
        self.guard
            .state
            .with_mut(|state| state.set_dirty(id, dirty));
    }

    /// Offers "Save" in the prompt. The handler should submit the page's form
    /// and report back via [`Self::finish_save`] or [`Self::save_failed`].
    pub(crate) fn set_save_handler(mut self, save: Callback<()>) {
        self.guard.save.set(Some((self.id, save)));
    }

    /// Marks the page as saved. Returns `true` if a navigation held back by the
    /// prompt was resumed, in which case the caller must not navigate itself.
    pub(crate) fn finish_save(mut self) -> bool {
        let id = self.id;
        let pending = self.guard.state.with_mut(|state| {
            let saving = state.saving;
            state.set_dirty(id, false);
            if saving { state.release() } else { None }
        });
        match pending {
            Some(pending) => {
                perform(pending);
                true
            }
            None => false,
        }
    }

    /// Cancels a save requested from the prompt so the user can fix the form.
    pub(crate) fn save_failed(self) {
        if self.guard.state.peek().saving {
            self.guard.keep_editing();
        }
    }
}

/// Guards the current page against navigation while it has unsaved changes.
pub(crate) fn use_page_navigation_guard() -> PageNavigationGuard {
    let guard = use_context::<NavigationGuard>();
    let page = use_hook(|| {
        let route = guard.inner_history().current_route();
        let mut state = guard.state;
        let id = state.with_mut(|state| state.register(route));
        PageNavigationGuard { guard, id }
    });
    use_drop(move || {
        let mut state = page.guard.state;
        state.with_mut(|state| state.unregister(page.id));
    });
    page
}

/// Renders the save/discard prompt for held-back navigation.
#[component]
pub(crate) fn NavigationGuardPrompt() -> Element {
    let guard = use_context::<NavigationGuard>();
    let traversal_pending = guard.state.read().pending == Some(PendingNavigation::BrowserTraversal);
    use_effect(use_reactive!(|traversal_pending| {
        if traversal_pending {
            guard.restore_after_traversal();
        }
    }));
    let open = guard.state.read().prompt_open();
    let can_save = guard.save_handler().is_some();

    rsx! {
        dxcomp::AlertDialog {
            open,
            on_open_change: move |open: bool| {
                if !open {
                    guard.keep_editing();
                }
            },
            dxcomp::AlertDialogTitle { "Save changes before leaving?" }
            dxcomp::AlertDialogDescription {
                "This page has unsaved changes. Leaving without saving will discard them."
            }
            dxcomp::AlertDialogActions {
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Outline,
                    onclick: move |_| guard.keep_editing(),
                    "Keep editing"
                }
                dxcomp::Button {
                    variant: dxcomp::ButtonVariant::Destructive,
                    onclick: move |_| guard.discard(),
                    "Discard changes"
                }
                if can_save {
                    dxcomp::Button {
                        onclick: move |_| guard.save(),
                        "Save"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GuardState, PendingNavigation};

    #[test]
    fn clean_pages_do_not_block_navigation() {
        let mut state = GuardState::default();
        assert!(!state.intercept(PendingNavigation::Back));

        state.register("/notes/create".to_string());
        assert!(!state.intercept(PendingNavigation::Push("/".to_string())));
        assert_eq!(state.pending, None);
    }

    #[test]
    fn dirty_pages_hold_back_the_latest_navigation() {
        let mut state = GuardState::default();
        let id = state.register("/entities/a/edit".to_string());
        state.set_dirty(id, true);

        assert!(state.intercept(PendingNavigation::Push("/".to_string())));
        assert!(state.intercept(PendingNavigation::Back));
        assert_eq!(state.pending, Some(PendingNavigation::Back));
        assert!(state.prompt_open());

        assert_eq!(state.release(), Some(PendingNavigation::Back));
        assert!(!state.is_blocking());
        assert!(!state.intercept(PendingNavigation::Back));
    }

    #[test]
    fn browser_traversal_waits_for_restore_before_prompting() {
        let mut state = GuardState::default();
        let id = state.register("/entities/a/edit".to_string());
        state.set_dirty(id, true);

        assert!(state.intercept(PendingNavigation::BrowserTraversal));
        assert!(!state.prompt_open());
    }

    #[test]
    fn saving_hides_the_prompt_but_keeps_the_pending_navigation() {
        let mut state = GuardState::default();
        let id = state.register("/entities/a/edit".to_string());
        state.set_dirty(id, true);
        state.intercept(PendingNavigation::Push("/".to_string()));
        state.saving = true;

        assert!(!state.prompt_open());
        assert_eq!(
            state.release(),
            Some(PendingNavigation::Push("/".to_string()))
        );
    }

    #[test]
    fn stale_registrations_do_not_affect_the_current_page() {
        let mut state = GuardState::default();
        let old = state.register("/entities/a/edit".to_string());
        let new = state.register("/entities/b/edit".to_string());

        state.set_dirty(old, true);
        assert!(!state.is_blocking());

        state.set_dirty(new, true);
        state.unregister(old);
        assert!(state.is_blocking());

        state.unregister(new);
        assert_eq!(state.entry, None);
    }
}
