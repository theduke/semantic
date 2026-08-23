use std::time::Duration;

use dioxus::prelude::*;

use super::state::{PlaybackIntent, PlaybackProgress};

#[component]
pub fn PlayerControls(
    queue_len: usize,
    active_index: Option<usize>,
    title: String,
    intent: PlaybackIntent,
    muted: bool,
    cycle: bool,
    playlist_open: bool,
    filter_open: bool,
    fullscreen: bool,
    image_interval: Option<Duration>,
    progress: PlaybackProgress,
    on_filter: EventHandler<MouseEvent>,
    on_playlist: EventHandler<MouseEvent>,
    on_mute: EventHandler<MouseEvent>,
    on_cycle: EventHandler<MouseEvent>,
    on_shuffle: EventHandler<MouseEvent>,
    on_fullscreen: EventHandler<MouseEvent>,
    on_previous: EventHandler<MouseEvent>,
    on_toggle_play: EventHandler<MouseEvent>,
    on_next: EventHandler<MouseEvent>,
    on_title: EventHandler<MouseEvent>,
    on_interval: EventHandler<Option<Duration>>,
    on_seek: EventHandler<f64>,
) -> Element {
    let has_active = active_index.is_some();
    let previous_disabled = active_index.is_none_or(|index| index == 0 && !cycle);
    let next_disabled = active_index.is_none_or(|index| index + 1 >= queue_len && !cycle);
    let position = active_index.map_or(0, |index| index + 1);
    let elapsed = format_time(progress.current_seconds);
    let duration = progress
        .duration_seconds
        .map(format_time)
        .unwrap_or_else(|| "--:--".to_string());
    let progress_max = progress.duration_seconds.unwrap_or(0.0).max(0.0);
    rsx! {
        div { class: "semantic-player__toolbar", role: "toolbar", aria_label: "Media controls",
            div { class: "semantic-player__toolbar-group",
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: toggle_variant(filter_open), onclick: on_filter,
                    aria_pressed: filter_open, aria_expanded: filter_open, aria_controls: "semantic-player-filter", "Filter" }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: toggle_variant(playlist_open), onclick: on_playlist,
                    aria_pressed: playlist_open, aria_expanded: playlist_open, aria_controls: "semantic-player-playlist", "Playlist" }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: toggle_variant(muted), onclick: on_mute,
                    aria_pressed: muted, title: if muted { "Unmute (M)" } else { "Mute (M)" }, if muted { "Muted" } else { "Sound" } }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: toggle_variant(cycle), onclick: on_cycle,
                    aria_pressed: cycle, title: "Cycle playlist", "Cycle" }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: dxcomp::ButtonVariant::Outline,
                    onclick: on_shuffle, disabled: queue_len < 2, title: "Shuffle all playlist items", "Shuffle" }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: toggle_variant(fullscreen), onclick: on_fullscreen,
                    aria_pressed: fullscreen, title: "Toggle fullscreen (F)", "Fullscreen" }
            }
            div { class: "semantic-player__transport",
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: dxcomp::ButtonVariant::Outline,
                    onclick: on_previous, disabled: previous_disabled, title: "Previous (Left arrow)", "Previous" }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, onclick: on_toggle_play, disabled: !has_active,
                    title: "Play or pause (Space)", if intent == PlaybackIntent::Playing { "Pause" } else { "Play" } }
                dxcomp::Button { size: dxcomp::ButtonSize::Sm, variant: dxcomp::ButtonVariant::Outline,
                    onclick: on_next, disabled: next_disabled, title: "Next (Right arrow)", "Next" }
            }
            div { class: "semantic-player__now-playing",
                span { class: "semantic-player__position", "{position} / {queue_len}" }
                button { class: "semantic-player__title", onclick: on_title, disabled: !has_active,
                    aria_label: "Open active entity: {title}", title: "{title}", "{title}" }
            }
            div { class: "semantic-player__progress",
                span { "{elapsed}" }
                input {
                    r#type: "range", min: "0", max: "{progress_max}", step: "0.1",
                    value: "{progress.current_seconds.min(progress_max)}",
                    disabled: progress_max <= 0.0,
                    aria_label: "Playback position",
                    onchange: move |event: FormEvent| {
                        if let Ok(seconds) = event.value().parse::<f64>() { on_seek.call(seconds); }
                    },
                }
                span { "{duration}" }
            }
            label { class: "semantic-player__interval",
                span { "Slides" }
                select {
                    value: interval_value(image_interval),
                    onchange: move |event: FormEvent| on_interval.call(parse_interval(&event.value())),
                    option { value: "manual", "Manual" }
                    option { value: "3", "3 sec" }
                    option { value: "5", "5 sec" }
                    option { value: "10", "10 sec" }
                    option { value: "30", "30 sec" }
                }
            }
        }
    }
}

fn toggle_variant(active: bool) -> dxcomp::ButtonVariant {
    if active {
        dxcomp::ButtonVariant::Primary
    } else {
        dxcomp::ButtonVariant::Outline
    }
}

fn interval_value(interval: Option<Duration>) -> String {
    interval
        .map(|value| value.as_secs().to_string())
        .unwrap_or_else(|| "manual".to_string())
}

fn parse_interval(value: &str) -> Option<Duration> {
    value.parse::<u64>().ok().map(Duration::from_secs)
}

fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
