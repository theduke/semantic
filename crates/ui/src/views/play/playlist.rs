use std::{collections::BTreeSet, rc::Rc};

use dioxus::prelude::*;

use super::state::QueueEntry;

const ROW_HEIGHT: u32 = 56;
const VIRTUALIZATION_THRESHOLD: usize = 200;

#[component]
pub fn PlayerPlaylist(
    entries: Rc<Vec<QueueEntry>>,
    active_index: Option<usize>,
    failed_occurrences: Rc<BTreeSet<u64>>,
    follow_active: bool,
    on_select: EventHandler<usize>,
    on_follow_change: EventHandler<bool>,
    on_close: EventHandler<MouseEvent>,
) -> Element {
    let entry_count = entries.len();
    let active_label =
        active_index.map_or_else(|| "None".to_string(), |index| (index + 1).to_string());
    let virtual_entries = entries.clone();
    let virtual_failed_occurrences = failed_occurrences.clone();
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
                dxcomp::Button {
                    size: dxcomp::ButtonSize::Xs,
                    variant: dxcomp::ButtonVariant::Outline,
                    onclick: on_close,
                    aria_controls: "semantic-player-playlist",
                    "Hide"
                }
            }
            if entry_count <= VIRTUALIZATION_THRESHOLD {
                div {
                    id: "semantic-player-virtual-list",
                    class: "semantic-player__playlist-scroll",
                    for index in 0..entry_count {
                        {playlist_row(
                            index,
                            &entries,
                            active_index,
                            &failed_occurrences,
                            on_select,
                        )}
                    }
                }
            } else {
                dxcomp::VirtualList {
                    id: "semantic-player-virtual-list",
                    class: "semantic-player__virtual-list",
                    count: entry_count,
                    buffer: 8_usize,
                    estimate_size: move |_| ROW_HEIGHT,
                    render_item: move |index: usize| playlist_row(
                        index,
                        &virtual_entries,
                        active_index,
                        &virtual_failed_occurrences,
                        on_select,
                    )
                }
            }
        }
    }
}

#[component]
pub fn PlaylistResizeHandle() -> Element {
    use_effect(move || {
        let _ = document::eval(
            r#"
            window.__semanticPlayerPlaylistResizeCleanup?.();

            const handle = document.getElementById('semantic-player-playlist-resizer');
            const body = document.getElementById('semantic-player-body');
            const playlist = document.getElementById('semantic-player-playlist');
            if (!handle || !body || !playlist) return;

            const abort = new AbortController();
            const options = { signal: abort.signal };
            let dragging = false;
            let pointerId = null;
            let bodyRect = null;

            const isStacked = () => window.matchMedia('(max-width: 720px)').matches;
            const clamp = (value, min, max) => Math.min(Math.max(value, min), max);
            const limits = (stacked) => {
                const extent = stacked ? bodyRect.height : bodyRect.width;
                const preferredMin = stacked ? 140 : 220;
                const reserved = stacked ? 120 : 180;
                const max = Math.max(96, Math.min(extent * 0.7, extent - reserved));
                return [Math.min(preferredMin, max), max];
            };
            const property = (stacked) => stacked
                ? '--semantic-player-playlist-height'
                : '--semantic-player-playlist-width';
            const storageKey = (stacked) => stacked
                ? 'semantic:player:playlist-height:v1'
                : 'semantic:player:playlist-width:v1';

            function currentSize(stacked) {
                const value = Number.parseFloat(body.style.getPropertyValue(property(stacked)));
                if (Number.isFinite(value)) return value;
                const rect = playlist.getBoundingClientRect();
                return stacked ? rect.height : rect.width;
            }

            function setSize(stacked, value, persist) {
                bodyRect = body.getBoundingClientRect();
                const [min, max] = limits(stacked);
                const size = clamp(value, min, max);
                body.style.setProperty(property(stacked), `${size}px`);
                handle.setAttribute('aria-valuemin', String(Math.round(min)));
                handle.setAttribute('aria-valuemax', String(Math.round(max)));
                handle.setAttribute('aria-valuenow', String(Math.round(size)));
                if (persist) {
                    try { localStorage.setItem(storageKey(stacked), String(size)); } catch (_) {}
                }
            }

            function updateOrientation() {
                const stacked = isStacked();
                handle.setAttribute('aria-orientation', stacked ? 'horizontal' : 'vertical');
                handle.title = stacked
                    ? 'Resize playlist (Up/Down arrows)'
                    : 'Resize playlist (Left/Right arrows)';
            }

            for (const stacked of [false, true]) {
                try {
                    const saved = Number.parseFloat(localStorage.getItem(storageKey(stacked)));
                    if (Number.isFinite(saved)) setSize(stacked, saved, false);
                } catch (_) {}
            }
            updateOrientation();

            handle.addEventListener('pointerdown', (event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                dragging = true;
                pointerId = event.pointerId;
                bodyRect = body.getBoundingClientRect();
                body.classList.add('semantic-player__body--resizing');
                handle.setPointerCapture(pointerId);
            }, options);

            handle.addEventListener('pointermove', (event) => {
                if (!dragging || event.pointerId !== pointerId) return;
                const stacked = isStacked();
                const size = stacked
                    ? bodyRect.bottom - event.clientY
                    : bodyRect.right - event.clientX;
                setSize(stacked, size, false);
            }, options);

            function stopDragging(event) {
                if (!dragging || event.pointerId !== pointerId) return;
                const stacked = isStacked();
                dragging = false;
                body.classList.remove('semantic-player__body--resizing');
                setSize(stacked, currentSize(stacked), true);
                if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
                pointerId = null;
            }
            handle.addEventListener('pointerup', stopDragging, options);
            handle.addEventListener('pointercancel', stopDragging, options);

            handle.addEventListener('keydown', (event) => {
                const stacked = isStacked();
                let delta = 0;
                if (stacked && event.key === 'ArrowUp') delta = 24;
                if (stacked && event.key === 'ArrowDown') delta = -24;
                if (!stacked && event.key === 'ArrowLeft') delta = 24;
                if (!stacked && event.key === 'ArrowRight') delta = -24;
                if (delta === 0) return;
                event.preventDefault();
                setSize(stacked, currentSize(stacked) + delta, true);
            }, options);

            window.addEventListener('resize', updateOrientation, options);
            window.__semanticPlayerPlaylistResizeCleanup = () => {
                abort.abort();
                body.classList.remove('semantic-player__body--resizing');
            };
        "#,
        );
    });
    use_drop(|| {
        _ = document::eval(
            r#"
            window.__semanticPlayerPlaylistResizeCleanup?.();
            delete window.__semanticPlayerPlaylistResizeCleanup;
        "#,
        );
    });

    rsx! {
        div {
            id: "semantic-player-playlist-resizer",
            class: "semantic-player__playlist-resizer",
            role: "separator",
            tabindex: 0,
            aria_controls: "semantic-player-playlist",
        }
    }
}

fn playlist_row(
    index: usize,
    entries: &[QueueEntry],
    active_index: Option<usize>,
    failed_occurrences: &BTreeSet<u64>,
    on_select: EventHandler<usize>,
) -> Element {
    let Some(entry) = entries.get(index).cloned() else {
        return rsx! {};
    };
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

pub fn scroll_to_index(index: usize) {
    spawn(async move {
        let eval = document::eval(
            r#"
            const index = await dioxus.recv();
            const element = document.getElementById('semantic-player-virtual-list');
            if (!element) return;

            const rowHeight = 56;
            const rowTop = index * rowHeight;
            const rowBottom = rowTop + rowHeight;
            const viewportTop = element.scrollTop;
            const viewportBottom = viewportTop + element.clientHeight;

            if (rowBottom > viewportBottom) {
                element.scrollTop = rowBottom - element.clientHeight;
            } else if (rowTop < viewportTop) {
                element.scrollTop = rowTop;
            }
        "#,
        );
        let _ = eval.send(index);
    });
}

fn format_duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
