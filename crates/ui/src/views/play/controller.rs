use dioxus::prelude::*;
use semantic_ui_core::{MediaHandleRegistration, MediaPlaybackEvent, MediaPlaybackEventKind};

use super::state::{PlaybackIntent, PlaybackProgress, PlayerState, QueueEntry};

#[derive(Clone, Copy)]
pub struct PlayerController {
    pub state: Signal<PlayerState>,
    pub progress: Signal<PlaybackProgress>,
    pub handle: Signal<Option<MediaHandleRegistration>>,
}

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
        if self.state.read().active_index == Some(index) {
            return;
        }
        self.pause_current();
        self.handle.set(None);
        self.state.write().select(index);
        self.progress.set(PlaybackProgress::default());
    }

    pub fn next(mut self) {
        let session = self.state.read().playback_session;
        self.pause_current();
        self.state.write().next();
        if self.state.read().playback_session != session {
            self.handle.set(None);
            self.progress.set(PlaybackProgress::default());
        }
    }

    pub fn previous(mut self) {
        let session = self.state.read().playback_session;
        self.pause_current();
        self.state.write().previous();
        if self.state.read().playback_session != session {
            self.handle.set(None);
            self.progress.set(PlaybackProgress::default());
        }
    }

    pub fn toggle_play(mut self) {
        let was_playing = self.state.read().playback_intent == PlaybackIntent::Playing;
        self.state.write().toggle_play();
        if was_playing {
            self.pause_current();
        } else {
            self.play_current();
        }
    }

    pub fn toggle_mute(mut self) {
        let muted = !self.state.read().muted;
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
        let state = self.state.read();
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
        if !self.state.read().event_is_current(&event) {
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
                    .read()
                    .active_entry()
                    .and_then(|entry| entry.known_duration_seconds);
                self.progress.write().duration_seconds = duration_seconds.or(known_duration);
            }
            _ => {}
        }
        let old_session = self.state.read().playback_session;
        self.state.write().apply_media_event(event);
        if self.state.read().playback_session != old_session {
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
        if self.state.read().playback_intent == PlaybackIntent::Playing {
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
        let old_session = self.state.read().playback_session;
        if self.state.read().active_index == Some(index) {
            self.pause_current();
        }
        self.state.write().remove(index);
        if self.state.read().playback_session != old_session {
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

    pub fn move_entry(mut self, index: usize, new_index: usize) {
        self.state.write().move_entry(index, new_index);
    }

    fn valid_handle(self) -> Option<MediaHandleRegistration> {
        let registration = self.handle.read().clone()?;
        let state = self.state.read();
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
