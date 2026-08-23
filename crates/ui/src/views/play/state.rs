use std::{collections::BTreeSet, rc::Rc, time::Duration};

use semantic_ui_core::{EntityTarget, MediaKind, MediaPlaybackEvent, MediaPlaybackEventKind};

#[derive(Clone, Debug, PartialEq)]
pub struct QueueEntry {
    pub occurrence_id: u64,
    pub target: EntityTarget,
    pub title: String,
    pub class_id: Option<String>,
    pub media_kind: MediaKind,
    pub mime_type: Option<String>,
    pub known_duration_seconds: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackIntent {
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedMediaState {
    Idle,
    Loading,
    Ready,
    Playing,
    Paused,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlaybackProgress {
    pub current_seconds: f64,
    pub duration_seconds: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerState {
    pub queue: Rc<Vec<QueueEntry>>,
    pub queue_generation: u64,
    pub active_index: Option<usize>,
    pub playback_session: u64,
    pub playback_intent: PlaybackIntent,
    pub observed_media_state: ObservedMediaState,
    pub muted: bool,
    pub cycle: bool,
    pub image_interval: Option<Duration>,
    pub image_remaining: Option<Duration>,
    pub progress: PlaybackProgress,
    pub playlist_open: bool,
    pub filter_open: bool,
    pub entity_dialog_open: bool,
    pub resume_after_dialog: bool,
    pub fullscreen: bool,
    pub transient_error: Option<String>,
    pub failed_occurrences: Rc<BTreeSet<u64>>,
    pub consecutive_failures: usize,
    next_occurrence_id: u64,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            queue: Rc::new(Vec::new()),
            queue_generation: 0,
            active_index: None,
            playback_session: 0,
            playback_intent: PlaybackIntent::Paused,
            observed_media_state: ObservedMediaState::Idle,
            muted: false,
            cycle: false,
            image_interval: Some(Duration::from_secs(5)),
            image_remaining: None,
            progress: PlaybackProgress::default(),
            playlist_open: true,
            filter_open: false,
            entity_dialog_open: false,
            resume_after_dialog: false,
            fullscreen: false,
            transient_error: None,
            failed_occurrences: Rc::new(BTreeSet::new()),
            consecutive_failures: 0,
            next_occurrence_id: 1,
        }
    }
}

impl PlayerState {
    pub fn active_entry(&self) -> Option<&QueueEntry> {
        self.active_index.and_then(|index| self.queue.get(index))
    }

    pub fn replace(&mut self, entries: Vec<QueueEntry>) {
        self.queue_generation = self.queue_generation.wrapping_add(1);
        let entries = self.assign_occurrences(entries);
        self.queue = Rc::new(entries);
        self.failed_occurrences = Rc::new(BTreeSet::new());
        self.active_index = (!self.queue.is_empty()).then_some(0);
        self.switch_session();
    }

    pub fn append(&mut self, entries: Vec<QueueEntry>) {
        if entries.is_empty() {
            return;
        }
        let was_empty = self.queue.is_empty();
        let mut queue = self.queue.as_ref().clone();
        queue.extend(self.assign_occurrences(entries));
        self.queue = Rc::new(queue);
        if was_empty {
            self.active_index = Some(0);
            self.switch_session();
        }
    }

    pub fn select(&mut self, index: usize) {
        if index >= self.queue.len() || self.active_index == Some(index) {
            return;
        }
        self.active_index = Some(index);
        self.switch_session();
    }

    pub fn next(&mut self) {
        let Some(index) = self.active_index else {
            return;
        };
        if index + 1 < self.queue.len() {
            self.active_index = Some(index + 1);
            self.switch_session();
        } else if self.cycle && !self.queue.is_empty() {
            self.active_index = Some(0);
            self.switch_session();
        } else {
            self.playback_intent = PlaybackIntent::Paused;
            self.observed_media_state = ObservedMediaState::Paused;
        }
    }

    pub fn previous(&mut self) {
        let Some(index) = self.active_index else {
            return;
        };
        if index > 0 {
            self.active_index = Some(index - 1);
            self.switch_session();
        } else if self.cycle && !self.queue.is_empty() {
            self.active_index = Some(self.queue.len() - 1);
            self.switch_session();
        }
    }

    pub fn play(&mut self) {
        if self.active_index.is_none() {
            return;
        }
        self.playback_intent = PlaybackIntent::Playing;
        if self
            .active_entry()
            .is_some_and(|entry| entry.media_kind == MediaKind::Image)
        {
            if self.image_remaining.is_none() {
                self.image_remaining = self.image_interval;
            }
            if self.observed_media_state == ObservedMediaState::Paused {
                self.observed_media_state = ObservedMediaState::Ready;
            }
        }
        self.transient_error = None;
    }

    pub fn pause(&mut self) {
        self.playback_intent = PlaybackIntent::Paused;
        self.observed_media_state = ObservedMediaState::Paused;
    }

    pub fn toggle_play(&mut self) {
        match self.playback_intent {
            PlaybackIntent::Playing => self.pause(),
            PlaybackIntent::Paused => self.play(),
        }
    }

    pub fn tick_image(&mut self, elapsed: Duration, session_id: u64) {
        if session_id != self.playback_session
            || self.playback_intent != PlaybackIntent::Playing
            || self.observed_media_state != ObservedMediaState::Ready
            || !self
                .active_entry()
                .is_some_and(|entry| entry.media_kind == MediaKind::Image)
        {
            return;
        }
        let Some(remaining) = self.image_remaining else {
            return;
        };
        if elapsed >= remaining {
            self.next();
        } else {
            self.image_remaining = Some(remaining - elapsed);
        }
    }

    pub fn set_image_interval(&mut self, interval: Option<Duration>) {
        self.image_interval = interval;
        self.image_remaining = match (self.image_remaining, interval) {
            (_, None) => None,
            (Some(remaining), Some(interval)) => Some(remaining.min(interval)),
            (None, Some(interval)) => Some(interval),
        };
    }

    pub fn apply_media_event(&mut self, event: MediaPlaybackEvent) {
        let Some((active_occurrence, active_duration, active_kind)) =
            self.active_entry().map(|active| {
                (
                    active.occurrence_id,
                    active.known_duration_seconds,
                    active.media_kind,
                )
            })
        else {
            return;
        };
        if event.session_id != self.playback_session || event.occurrence_id != active_occurrence {
            return;
        }
        match event.kind {
            MediaPlaybackEventKind::Mounted => {
                self.observed_media_state = ObservedMediaState::Loading
            }
            MediaPlaybackEventKind::Loaded { duration_seconds } => {
                let mut failed = self.failed_occurrences.as_ref().clone();
                failed.remove(&active_occurrence);
                self.failed_occurrences = Rc::new(failed);
                self.observed_media_state = ObservedMediaState::Ready;
                self.progress.duration_seconds = duration_seconds.or(active_duration);
                if active_kind == MediaKind::Image {
                    self.image_remaining = self.image_interval;
                }
            }
            MediaPlaybackEventKind::Playing => {
                self.observed_media_state = ObservedMediaState::Playing;
                self.playback_intent = PlaybackIntent::Playing;
                self.consecutive_failures = 0;
            }
            MediaPlaybackEventKind::Paused => {
                self.observed_media_state = ObservedMediaState::Paused;
                self.playback_intent = PlaybackIntent::Paused;
            }
            MediaPlaybackEventKind::Progress {
                current_seconds,
                duration_seconds,
            } => {
                self.progress.current_seconds = current_seconds.max(0.0);
                if duration_seconds.is_some() {
                    self.progress.duration_seconds = duration_seconds;
                }
            }
            MediaPlaybackEventKind::Finished => {
                self.consecutive_failures = 0;
                self.next();
            }
            MediaPlaybackEventKind::Failed { message, fatal } => {
                self.observed_media_state = ObservedMediaState::Failed;
                let mut failed = self.failed_occurrences.as_ref().clone();
                failed.insert(active_occurrence);
                self.failed_occurrences = Rc::new(failed);
                self.consecutive_failures += 1;
                if fatal
                    && self.playback_intent == PlaybackIntent::Playing
                    && self.consecutive_failures < self.queue.len().clamp(1, 8)
                {
                    self.next();
                    self.transient_error = Some(format!("Skipped failed media: {message}"));
                } else {
                    self.transient_error = Some(message);
                    self.playback_intent = PlaybackIntent::Paused;
                }
            }
            MediaPlaybackEventKind::PlayRejected { message } => {
                self.playback_intent = PlaybackIntent::Paused;
                self.observed_media_state = ObservedMediaState::Paused;
                self.transient_error = Some(format!(
                    "Playback was blocked: {message}. Press play to continue."
                ));
            }
        }
    }

    pub fn open_dialog(&mut self) {
        if self.entity_dialog_open {
            return;
        }
        self.resume_after_dialog = self.playback_intent == PlaybackIntent::Playing;
        self.entity_dialog_open = true;
        self.pause();
    }

    pub fn close_dialog(&mut self) {
        if !self.entity_dialog_open {
            return;
        }
        self.entity_dialog_open = false;
        if self.resume_after_dialog && self.observed_media_state != ObservedMediaState::Failed {
            self.play();
        }
        self.resume_after_dialog = false;
    }

    pub fn shuffle(&mut self) {
        if self.queue.len() < 2 {
            return;
        }
        let active_occurrence = self.active_entry().map(|entry| entry.occurrence_id);
        let active_position = self.active_index.unwrap_or(0);
        let mut queue = self.queue.as_ref().clone();
        let mut seed = self.queue_generation ^ self.playback_session ^ queue.len() as u64;
        for index in (1..queue.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            queue.swap(index, seed as usize % (index + 1));
        }
        if let Some(occurrence) = active_occurrence {
            if let Some(shuffled_index) = queue
                .iter()
                .position(|entry| entry.occurrence_id == occurrence)
            {
                queue.swap(active_position, shuffled_index);
            }
        }
        self.queue = Rc::new(queue);
    }

    pub fn remove_active(&mut self) {
        let Some(index) = self.active_index else {
            return;
        };
        let mut queue = self.queue.as_ref().clone();
        let removed = queue.remove(index);
        self.queue = Rc::new(queue);
        let mut failed = self.failed_occurrences.as_ref().clone();
        failed.remove(&removed.occurrence_id);
        self.failed_occurrences = Rc::new(failed);
        self.active_index = if self.queue.is_empty() {
            None
        } else {
            Some(index.min(self.queue.len() - 1))
        };
        self.switch_session();
    }

    fn switch_session(&mut self) {
        self.playback_session = self.playback_session.wrapping_add(1);
        self.progress = PlaybackProgress::default();
        if self.active_index.is_none() {
            self.playback_intent = PlaybackIntent::Paused;
        }
        self.observed_media_state = if self.active_index.is_some() {
            ObservedMediaState::Loading
        } else {
            ObservedMediaState::Idle
        };
        self.image_remaining = self.image_interval;
        self.transient_error = None;
    }

    fn assign_occurrences(&mut self, mut entries: Vec<QueueEntry>) -> Vec<QueueEntry> {
        for entry in &mut entries {
            entry.occurrence_id = self.next_occurrence_id;
            self.next_occurrence_id = self.next_occurrence_id.wrapping_add(1).max(1);
        }
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, kind: MediaKind) -> QueueEntry {
        QueueEntry {
            occurrence_id: 0,
            target: EntityTarget::default_collection(id),
            title: id.to_string(),
            class_id: None,
            media_kind: kind,
            mime_type: None,
            known_duration_seconds: None,
        }
    }

    #[test]
    fn boundaries_and_cycle_are_safe() {
        let mut state = PlayerState::default();
        state.next();
        state.previous();
        state.play();
        assert_eq!(state.active_index, None);
        state.replace(vec![
            entry("a", MediaKind::Audio),
            entry("b", MediaKind::Video),
        ]);
        state.previous();
        assert_eq!(state.active_index, Some(0));
        state.next();
        state.next();
        assert_eq!(state.active_index, Some(1));
        assert_eq!(state.playback_intent, PlaybackIntent::Paused);
        state.cycle = true;
        state.next();
        assert_eq!(state.active_index, Some(0));
        state.previous();
        assert_eq!(state.active_index, Some(1));
    }

    #[test]
    fn stale_events_do_not_change_the_new_item() {
        let mut state = PlayerState::default();
        state.replace(vec![
            entry("a", MediaKind::Audio),
            entry("b", MediaKind::Audio),
        ]);
        let session = state.playback_session;
        let occurrence = state.active_entry().unwrap().occurrence_id;
        state.next();
        state.apply_media_event(MediaPlaybackEvent {
            session_id: session,
            occurrence_id: occurrence,
            kind: MediaPlaybackEventKind::Finished,
        });
        assert_eq!(state.active_index, Some(1));
    }

    #[test]
    fn image_play_and_pause_preserve_the_countdown() {
        let mut state = PlayerState::default();
        state.replace(vec![
            entry("a", MediaKind::Image),
            entry("b", MediaKind::Image),
        ]);
        state.observed_media_state = ObservedMediaState::Ready;
        state.play();
        let session = state.playback_session;
        state.tick_image(Duration::from_secs(2), session);
        state.pause();
        state.tick_image(Duration::from_secs(9), session);
        assert_eq!(state.image_remaining, Some(Duration::from_secs(3)));
        state.play();
        state.tick_image(Duration::from_secs(3), session);
        assert_eq!(state.active_index, Some(1));
    }

    #[test]
    fn append_keeps_active_and_duplicate_occurrences_are_distinct() {
        let mut state = PlayerState::default();
        state.replace(vec![entry("same", MediaKind::Audio)]);
        let first = state.active_entry().unwrap().occurrence_id;
        let session = state.playback_session;
        state.append(vec![entry("same", MediaKind::Audio)]);
        assert_eq!(state.playback_session, session);
        assert_ne!(first, state.queue[1].occurrence_id);
    }

    #[test]
    fn shuffle_is_a_complete_permutation_and_keeps_active_position() {
        let mut state = PlayerState::default();
        state.replace(
            (0..50)
                .map(|n| entry(&n.to_string(), MediaKind::Image))
                .collect(),
        );
        state.select(20);
        let active = state.active_entry().unwrap().occurrence_id;
        let mut before: Vec<_> = state
            .queue
            .iter()
            .map(|entry| entry.occurrence_id)
            .collect();
        state.shuffle();
        let mut after: Vec<_> = state
            .queue
            .iter()
            .map(|entry| entry.occurrence_id)
            .collect();
        before.sort_unstable();
        after.sort_unstable();
        assert_eq!(before, after);
        assert_eq!(state.queue[20].occurrence_id, active);
    }

    #[test]
    fn fifty_thousand_entry_queue_stays_lightweight_and_shuffleable() {
        let mut state = PlayerState::default();
        state.replace(
            (0..50_000)
                .map(|index| entry(&format!("media-{index}"), MediaKind::Image))
                .collect(),
        );
        state.select(40_000);
        let active = state.active_entry().unwrap().occurrence_id;
        state.shuffle();
        assert_eq!(state.queue.len(), 50_000);
        assert_eq!(state.queue[40_000].occurrence_id, active);
    }

    #[test]
    fn dialog_resumes_only_when_it_was_playing() {
        let mut state = PlayerState::default();
        state.replace(vec![entry("a", MediaKind::Audio)]);
        state.play();
        state.open_dialog();
        assert_eq!(state.playback_intent, PlaybackIntent::Paused);
        state.close_dialog();
        assert_eq!(state.playback_intent, PlaybackIntent::Playing);
        state.pause();
        state.open_dialog();
        state.close_dialog();
        assert_eq!(state.playback_intent, PlaybackIntent::Paused);
    }

    #[test]
    fn failed_items_are_marked_and_failure_loop_stops() {
        let mut state = PlayerState::default();
        state.replace(vec![
            entry("a", MediaKind::Audio),
            entry("b", MediaKind::Audio),
        ]);
        state.play();
        let first_session = state.playback_session;
        let first_occurrence = state.active_entry().unwrap().occurrence_id;
        state.apply_media_event(MediaPlaybackEvent {
            session_id: first_session,
            occurrence_id: first_occurrence,
            kind: MediaPlaybackEventKind::Failed {
                message: "broken".to_string(),
                fatal: true,
            },
        });
        assert!(state.failed_occurrences.contains(&first_occurrence));
        assert_eq!(state.active_index, Some(1));
        let second_session = state.playback_session;
        let second_occurrence = state.active_entry().unwrap().occurrence_id;
        state.apply_media_event(MediaPlaybackEvent {
            session_id: second_session,
            occurrence_id: second_occurrence,
            kind: MediaPlaybackEventKind::Failed {
                message: "also broken".to_string(),
                fatal: true,
            },
        });
        assert_eq!(state.playback_intent, PlaybackIntent::Paused);
        assert_eq!(state.active_index, Some(1));
    }

    #[test]
    fn removing_active_selects_a_valid_neighbor() {
        let mut state = PlayerState::default();
        state.replace(vec![
            entry("a", MediaKind::Image),
            entry("b", MediaKind::Image),
        ]);
        state.select(1);
        state.remove_active();
        assert_eq!(state.active_index, Some(0));
        assert_eq!(state.active_entry().unwrap().target.id, "a");
        state.remove_active();
        assert_eq!(state.active_index, None);
    }
}
