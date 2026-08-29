use std::{collections::BTreeSet, rc::Rc};

use dioxus::prelude::*;

use super::state::QueueEntry;

const ROW_HEIGHT: u32 = 56;

#[component]
pub fn PlayerPlaylist(
    entries: Rc<Vec<QueueEntry>>,
    active_index: Option<usize>,
    failed_occurrences: Rc<BTreeSet<u64>>,
    follow_active: bool,
    on_select: EventHandler<usize>,
    on_follow_change: EventHandler<bool>,
) -> Element {
    let mut count = use_signal(|| entries.len());
    let entry_count = entries.len();
    use_effect(use_reactive((&entry_count,), move |(entry_count,)| {
        count.set(entry_count);
    }));
    let active_label =
        active_index.map_or_else(|| "None".to_string(), |index| (index + 1).to_string());
    rsx! {
        aside { id: "semantic-player-playlist", class: "semantic-player__playlist", aria_label: "Playlist",
            header { class: "semantic-player__playlist-header",
                strong { "Playlist" }
                span { "{entries.len()} items" }
                label {
                    input {
                        r#type: "checkbox", checked: follow_active,
                        onchange: move |event| on_follow_change.call(event.checked()),
                    }
                    "Follow"
                }
                dxcomp::Button {
                    size: dxcomp::ButtonSize::Xs,
                    variant: dxcomp::ButtonVariant::Outline,
                    disabled: active_index.is_none(),
                    onclick: move |_| {
                        if let Some(index) = active_index { scroll_to_index(index); }
                    },
                    "Current {active_label}"
                }
            }
            div { class: "semantic-player__playlist-scroll",
                dxcomp::VirtualList {
                    id: "semantic-player-virtual-list",
                    class: "semantic-player__virtual-list",
                    count,
                    buffer: 8_usize,
                    estimate_size: move |_| ROW_HEIGHT,
                    render_item: move |index: usize| {
                        let Some(entry) = entries.get(index).cloned() else { return rsx! {}; };
                        let active = active_index == Some(index);
                        let failed = failed_occurrences.contains(&entry.occurrence_id);
                        let kind = format!("{:?}", entry.media_kind);
                        let duration = entry.known_duration_seconds.map(format_duration);
                        rsx! {
                            button {
                                key: "{entry.occurrence_id}",
                                class: if failed { "semantic-player__playlist-row semantic-player__playlist-row--failed" } else if active { "semantic-player__playlist-row semantic-player__playlist-row--active" } else { "semantic-player__playlist-row" },
                                aria_current: active.then_some("true"),
                                aria_label: "Play {entry.title}",
                                onclick: move |_| on_select.call(index),
                                span { class: "semantic-player__playlist-number", "{index + 1}" }
                                span { class: "semantic-player__playlist-title", title: "{entry.title}", "{entry.title}" }
                                span { class: "semantic-player__playlist-kind", "{kind}" }
                                if let Some(duration) = duration { span { "{duration}" } }
                                if failed { span { class: "semantic-player__playlist-failed", title: "Playback failed", "Failed" } }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn scroll_to_index(index: usize) {
    spawn(async move {
        let eval = document::eval(
            r#"
            const index = await dioxus.recv();
            const element = document.getElementById('semantic-player-virtual-list');
            if (element) element.scrollTop = index * 56;
        "#,
        );
        let _ = eval.send(index);
    });
}

fn format_duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
