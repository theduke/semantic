use dioxus::prelude::*;
use semantic_ui_core::{MediaHandleRegistration, MediaPlaybackEvent, MediaPlaybackEventKind};

use super::state::{PlaybackIntent, PlaybackProgress, PlayerState, QueueEntry};

#[derive(Clone, Copy)]
pub struct PlayerController {
    pub state: Signal<PlayerState>,
    pub progress: Signal<PlaybackProgress>,
    pub handle: Signal<Option<MediaHandleRegistration>>,
}

// These imperative callbacks also run inside media mount effects. Keep reads
// untracked so state updates cannot subscribe and rerun those effects.
impl PlayerController {
    pub fn replace(mut self, entries: Vec<QueueEntry>) {
        self.pause_current();
        self.handle.set(None);
        self.state.write().replace(entries);
        self.progress.set(PlaybackProgress::default());
    }

    pub fn append(mut self, entries: Vec<QueueEntry>) {
        self.state.write().append(entries);
    }

    pub fn select(mut self, index: usize) {
        if self.state.peek().active_index == Some(index) {
            return;
        }
        self.pause_current();
        self.handle.set(None);
        self.state.write().select(index);
        self.progress.set(PlaybackProgress::default());
    }

    pub fn next(mut self) {
        let session = self.state.peek().playback_session;
        self.pause_current();
        self.state.write().next();
        if self.state.peek().playback_session != session {
            self.handle.set(None);
            self.progress.set(PlaybackProgress::default());
        }
    }

    pub fn previous(mut self) {
        let session = self.state.peek().playback_session;
        self.pause_current();
        self.state.write().previous();
        if self.state.peek().playback_session != session {
            self.handle.set(None);
            self.progress.set(PlaybackProgress::default());
        }
    }

    pub fn toggle_play(mut self) {
        let was_playing = self.state.peek().playback_intent == PlaybackIntent::Playing;
        self.state.write().toggle_play();
        if was_playing {
            self.pause_current();
        } else {
            self.play_current();
        }
    }

    pub fn toggle_mute(mut self) {
        let muted = !self.state.peek().muted;
        self.state.write().muted = muted;
        if let Some(registration) = self.valid_handle() {
            if let Some(handle) = registration.handle {
                handle.0.set_muted(muted);
            }
        }
    }

    pub fn seek(mut self, seconds: f64) {
        if let Some(registration) = self.valid_handle() {
            if let Some(handle) = registration.handle {
                handle.0.seek(seconds);
                self.progress.write().current_seconds = seconds.max(0.0);
            }
        }
    }

    pub fn register_handle(mut self, registration: MediaHandleRegistration) {
        let state = self.state.peek();
        let valid = state.playback_session == registration.session_id
            && state
                .active_entry()
                .is_some_and(|entry| entry.occurrence_id == registration.occurrence_id);
        let muted = state.muted;
        let playing = state.playback_intent == PlaybackIntent::Playing;
        drop(state);
        if !valid {
            return;
        }
        if let Some(handle) = &registration.handle {
            handle.0.set_muted(muted);
            if playing {
                handle.0.play();
            }
        }
        let has_handle = registration.handle.is_some();
        self.handle.set(has_handle.then_some(registration));
    }

    pub fn media_event(mut self, event: MediaPlaybackEvent) {
        if !self.state.peek().event_is_current(&event) {
            return;
        }
        match &event.kind {
            MediaPlaybackEventKind::Progress {
                current_seconds,
                duration_seconds,
            } => {
                let mut progress = self.progress.write();
                progress.current_seconds = current_seconds.max(0.0);
                if duration_seconds.is_some() {
                    progress.duration_seconds = *duration_seconds;
                }
                return;
            }
            MediaPlaybackEventKind::Loaded { duration_seconds } => {
                let known_duration = self
                    .state
                    .peek()
                    .active_entry()
                    .and_then(|entry| entry.known_duration_seconds);
                self.progress.write().duration_seconds = duration_seconds.or(known_duration);
            }
            _ => {}
        }
        let old_session = self.state.peek().playback_session;
        self.state.write().apply_media_event(event);
        if self.state.peek().playback_session != old_session {
            self.handle.set(None);
            self.progress.set(PlaybackProgress::default());
        }
    }

    pub fn open_dialog(mut self) {
        self.state.write().open_dialog();
        self.pause_current();
    }
    pub fn close_dialog(mut self) {
        self.state.write().close_dialog();
        if self.state.peek().playback_intent == PlaybackIntent::Playing {
            self.play_current();
        }
    }

    pub fn remove_active(mut self) {
        self.pause_current();
        self.handle.set(None);
        self.state.write().remove_active();
        self.progress.set(PlaybackProgress::default());
    }

    pub fn retry_current(mut self) {
        self.pause_current();
        self.handle.set(None);
        self.state.write().retry_current();
        self.progress.set(PlaybackProgress::default());
    }

    pub fn remove(mut self, index: usize) {
        let old_session = self.state.peek().playback_session;
        if self.state.peek().active_index == Some(index) {
            self.pause_current();
        }
        self.state.write().remove(index);
        if self.state.peek().playback_session != old_session {
            self.handle.set(None);
            self.progress.set(PlaybackProgress::default());
        }
    }

    pub fn clear(mut self) {
        self.pause_current();
        self.handle.set(None);
        self.state.write().clear();
        self.progress.set(PlaybackProgress::default());
    }

    fn valid_handle(self) -> Option<MediaHandleRegistration> {
        let registration = self.handle.peek().clone()?;
        let state = self.state.peek();
        (registration.session_id == state.playback_session
            && state
                .active_entry()
                .is_some_and(|entry| entry.occurrence_id == registration.occurrence_id))
        .then_some(registration)
    }

    fn play_current(self) {
        if let Some(registration) = self.valid_handle() {
            if let Some(handle) = registration.handle {
                handle.0.play();
            }
        }
    }

    fn pause_current(self) {
        if let Some(registration) = self.valid_handle() {
            if let Some(handle) = registration.handle {
                handle.0.pause();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use dioxus::dioxus_core::ReactiveContext;
    use futures::{FutureExt, StreamExt};
    use semantic_ui_core::{
        EntityTarget, MediaKind, PlaybackMediaHandle, RegisteredPlaybackHandle,
    };

    use super::*;
    use crate::views::play::state::ObservedMediaState;

    fn with_controller(kind: MediaKind, test: impl FnOnce(PlayerController)) {
        let mut dom = VirtualDom::new(|| rsx! {});
        dom.rebuild_in_place();
        dom.in_scope(ScopeId::ROOT, || {
            let mut state = PlayerState::default();
            state.replace(vec![QueueEntry {
                occurrence_id: 0,
                target: EntityTarget::default_collection("media"),
                title: "Media".to_string(),
                class_id: None,
                media_kind: kind,
                mime_type: None,
                known_duration_seconds: None,
            }]);
            test(PlayerController {
                state: Signal::new(state),
                progress: Signal::new(PlaybackProgress::default()),
                handle: Signal::new(None),
            });
        });
    }

    #[test]
    fn media_events_do_not_resubscribe_mount_effect() {
        with_controller(MediaKind::Image, |mut controller| {
            let session_id = controller.state.peek().playback_session;
            let occurrence_id = controller
                .state
                .peek()
                .active_entry()
                .unwrap()
                .occurrence_id;
            let on_event = EventHandler::new(move |kind| {
                controller.media_event(MediaPlaybackEvent {
                    session_id,
                    occurrence_id,
                    kind,
                });
            });
            let (mount_effect, mut updates) = ReactiveContext::new();
            mount_effect.run_in(|| on_event.call(MediaPlaybackEventKind::Mounted));
            assert!(updates.next().now_or_never().is_none());

            on_event.call(MediaPlaybackEventKind::Loaded {
                duration_seconds: None,
            });
            controller.toggle_play();
            controller.state.write().playlist_open = false;

            assert!(updates.next().now_or_never().is_none());
            assert_eq!(
                controller.state.peek().observed_media_state,
                ObservedMediaState::Ready
            );
            let remaining = controller.state.peek().image_remaining.unwrap();
            controller.state.write().tick_image(remaining, session_id);
            assert_eq!(
                controller.state.peek().playback_intent,
                PlaybackIntent::Paused
            );
        });
    }

    #[test]
    fn missing_source_failure_does_not_resubscribe_effect() {
        with_controller(MediaKind::Image, |mut controller| {
            controller.toggle_play();
            let event = MediaPlaybackEvent {
                session_id: controller.state.peek().playback_session,
                occurrence_id: controller
                    .state
                    .peek()
                    .active_entry()
                    .unwrap()
                    .occurrence_id,
                kind: MediaPlaybackEventKind::Failed {
                    message: "This image has no usable source".to_string(),
                    fatal: true,
                },
            };
            let on_event = EventHandler::new(move |event| controller.media_event(event));
            let (missing_source_effect, mut updates) = ReactiveContext::new();
            missing_source_effect.run_in(|| on_event.call(event));
            controller.state.write().playlist_open = false;

            assert!(updates.next().now_or_never().is_none());
            assert_eq!(controller.state.peek().consecutive_failures, 1);
            assert_eq!(
                controller.state.peek().playback_intent,
                PlaybackIntent::Paused
            );
        });
    }

    #[derive(Default)]
    struct TestHandle {
        plays: Cell<usize>,
        muted: Cell<bool>,
    }

    impl PlaybackMediaHandle for TestHandle {
        fn play(&self) {
            self.plays.set(self.plays.get() + 1);
        }

        fn pause(&self) {}

        fn set_muted(&self, muted: bool) {
            self.muted.set(muted);
        }

        fn seek(&self, _seconds: f64) {}
    }

    #[test]
    fn handle_registration_does_not_resubscribe_mount_effect() {
        with_controller(MediaKind::Audio, |mut controller| {
            controller.toggle_play();
            let handle = Rc::new(TestHandle::default());
            let registration = MediaHandleRegistration {
                session_id: controller.state.peek().playback_session,
                occurrence_id: controller
                    .state
                    .peek()
                    .active_entry()
                    .unwrap()
                    .occurrence_id,
                handle: Some(RegisteredPlaybackHandle(handle.clone())),
            };
            let on_handle = EventHandler::new(move |registration| {
                controller.register_handle(registration);
            });
            let (mount_effect, mut updates) = ReactiveContext::new();
            mount_effect.run_in(|| on_handle.call(registration));
            assert_eq!(handle.plays.get(), 1);

            let (command_context, mut command_updates) = ReactiveContext::new();
            command_context.run_in(|| {
                controller.toggle_mute();
                controller.seek(5.0);
                controller.toggle_play();
                controller.toggle_play();
            });
            controller.state.write().playlist_open = false;

            assert!(updates.next().now_or_never().is_none());
            assert_eq!(handle.plays.get(), 2);
            assert!(handle.muted.get());
            assert!(controller.handle.peek().is_some());
            assert_eq!(controller.progress.peek().current_seconds, 5.0);

            controller.handle.set(None);
            assert!(command_updates.next().now_or_never().is_none());
        });
    }
}
