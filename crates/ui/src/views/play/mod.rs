mod browser_bridge;
mod controller;
mod data;
mod entity_dialog;
mod filters;
mod playlist;
mod query;
mod stage;
mod state;

use browser_bridge::{PlayerShortcut, toggle_fullscreen, use_player_browser_bridge};
use controller::PlayerController;
use data::{get_object, load_playlist};
use dioxus::prelude::*;
use entity_dialog::PlayerEntityDialog;
use filters::PlayerFilters;
use playlist::{PlayerPlaylist, PlaylistResizeHandle};
use query::PlaylistFilter;
use semantic_ui_core::{MediaHandleRegistration, MediaKind, use_active_scope_id, use_rpc_client};
use stage::PlayerStage;
use state::{ObservedMediaState, PlaybackIntent, PlaybackProgress, PlayerState};

use self::controls::PlayerControls;
mod controls;

#[component]
pub fn PlayPage() -> Element {
    let client = use_rpc_client();
    let scope_id = use_active_scope_id();
    let mut state = use_signal(PlayerState::default);
    let progress = use_signal(PlaybackProgress::default);
    let handle = use_signal(|| None::<MediaHandleRegistration>);
    let controller = PlayerController {
        state,
        progress,
        handle,
    };
    let mut draft = use_signal(PlaylistFilter::default);
    let mut loading = use_signal(|| true);
    let mut playlist_loaded = use_signal(|| false);
    let mut initial_retry_revision = use_signal(|| 0_u64);
    let mut filter_error = use_signal(|| None::<String>);
    let mut queue_warning = use_signal(|| None::<String>);
    let mut request_generation = use_signal(|| 0_u64);
    let mut follow_active = use_signal(|| true);

    use_future(move || async move {
        let eval = document::eval(
            r#"return localStorage.getItem('semantic:player:playlist-open:v2') ?? localStorage.getItem('semantic:player:playlist-open:v1');"#,
        );
        if let Ok(value) = eval.await {
            if let Some(open) = value.as_str().map(|value| value != "false") {
                state.write().playlist_open = open;
            }
        }
    });

    let initial_load_key = use_memo(use_reactive(
        &(scope_id.clone(), *initial_retry_revision.read()),
        |key| key,
    ));
    use_future({
        let client = client.clone();
        move || {
            let client = client.clone();
            let (scope_id, _retry_revision) = initial_load_key();
            async move {
                loading.set(true);
                filter_error.set(None);
                request_generation += 1;
                let generation = *request_generation.read();
                match load_playlist(client, scope_id, PlaylistFilter::default()).await {
                    Ok(result) if generation == *request_generation.read() => {
                        controller.replace(result.entries);
                        queue_warning.set(result.warning);
                        playlist_loaded.set(true);
                    }
                    Err(error) if generation == *request_generation.read() => {
                        filter_error.set(Some(error))
                    }
                    _ => {}
                }
                if generation == *request_generation.read() {
                    loading.set(false);
                }
            }
        }
    });

    let active_scope = use_memo(use_reactive(&scope_id, |scope_id| scope_id));
    let active_request = use_memo(move || {
        let state = state.read();
        state.active_entry().map(|entry| {
            (
                active_scope(),
                state.queue_generation,
                state.playback_session,
                entry.target.clone(),
            )
        })
    });
    let mut active_object = use_resource({
        let client = client.clone();
        move || {
            let client = client.clone();
            let request = active_request();
            async move {
                match request {
                    Some((scope_id, _, _, target)) => {
                        Some(get_object(client, scope_id, target).await)
                    }
                    None => None,
                }
            }
        }
    });

    let image_deadline = use_memo(move || {
        let current = state.read();
        (current.playback_intent == PlaybackIntent::Playing
            && current.observed_media_state == ObservedMediaState::Ready
            && current
                .active_entry()
                .is_some_and(|entry| entry.media_kind == MediaKind::Image))
        .then(|| {
            current
                .image_remaining
                .map(|remaining| (current.playback_session, remaining))
        })
        .flatten()
    });
    let _image_timer = use_resource(move || {
        let deadline = image_deadline();
        async move {
            let Some((session, remaining)) = deadline else {
                return;
            };
            dioxus_sdk_time::sleep(remaining).await;
            state.write().tick_image(remaining, session);
        }
    });

    let shortcut_controller = controller;
    let mut shortcut_state = state;
    use_player_browser_bridge(
        true,
        EventHandler::new(move |shortcut| {
            let blocked = {
                let current = shortcut_state.read();
                current.filter_open || current.entity_dialog_open
            };
            if blocked && shortcut != PlayerShortcut::Escape {
                return;
            }
            match shortcut {
                PlayerShortcut::Previous => shortcut_controller.previous(),
                PlayerShortcut::Next => shortcut_controller.next(),
                PlayerShortcut::TogglePlay => shortcut_controller.toggle_play(),
                PlayerShortcut::ToggleMute => shortcut_controller.toggle_mute(),
                PlayerShortcut::ToggleFullscreen => toggle_fullscreen(),
                PlayerShortcut::Escape => {
                    if shortcut_state.read().filter_open {
                        shortcut_state.write().filter_open = false;
                    } else if shortcut_state.read().entity_dialog_open {
                        shortcut_controller.close_dialog();
                    } else if shortcut_state.read().fullscreen {
                        toggle_fullscreen();
                    }
                }
            }
        }),
        EventHandler::new(move |fullscreen| state.write().fullscreen = fullscreen),
    );

    let current = state.read().clone();
    let active_entry = current.active_entry().cloned();
    let active_result = active_object
        .read_unchecked()
        .as_ref()
        .and_then(|result| result.clone());
    let current_title = active_entry
        .as_ref()
        .map(|entry| entry.title.clone())
        .unwrap_or_else(|| "Nothing selected".to_string());
    let dialog_target = active_entry.as_ref().map(|entry| entry.target.clone());

    let replace_handler = load_handler(
        client.clone(),
        scope_id.clone(),
        false,
        controller,
        loading,
        filter_error,
        queue_warning,
        request_generation,
        state,
        playlist_loaded,
    );
    let append_handler = load_handler(
        client,
        scope_id,
        true,
        controller,
        loading,
        filter_error,
        queue_warning,
        request_generation,
        state,
        playlist_loaded,
    );

    rsx! {
        section { id: "semantic-player-root", class: "semantic-player",
            h1 { class: "semantic-visually-hidden", "Media player" }
            PlayerControls {
                queue_len: current.queue.len(), active_index: current.active_index,
                title: current_title.clone(), intent: current.playback_intent,
                muted: current.muted, cycle: current.cycle, playlist_open: current.playlist_open,
                filter_open: current.filter_open, fullscreen: current.fullscreen,
                image_interval: current.image_interval, progress,
                on_filter: move |_| { let open = !state.read().filter_open; state.write().filter_open = open; },
                on_playlist: move |_| {
                    let open = !state.read().playlist_open;
                    state.write().playlist_open = open;
                    store_playlist_open(open);
                },
                on_mute: move |_| controller.toggle_mute(),
                on_cycle: move |_| { let cycle = !state.read().cycle; state.write().cycle = cycle; },
                on_shuffle: move |_| state.write().shuffle(),
                on_fullscreen: move |_| toggle_fullscreen(),
                on_previous: move |_| controller.previous(),
                on_toggle_play: move |_| controller.toggle_play(),
                on_next: move |_| controller.next(),
                on_title: move |_| { if state.read().active_index.is_some() { controller.open_dialog(); } },
                on_interval: move |interval| state.write().set_image_interval(interval),
                on_seek: move |seconds| controller.seek(seconds),
            }
            if current.filter_open {
                PlayerFilters {
                    draft: draft(), loading: *loading.read(), error: filter_error(),
                    on_change: move |next| draft.set(next),
                    on_replace: replace_handler,
                    on_add: append_handler,
                    on_cancel: move |_| state.write().filter_open = false,
                }
            }
            if let Some(warning) = queue_warning() {
                div { class: "semantic-player__warning", role: "status", "{warning}" }
            }
                    div {
                        id: "semantic-player-body",
                        class: if current.playlist_open { "semantic-player__body semantic-player__body--playlist" } else { "semantic-player__body" },
                        div { class: "semantic-player__stage-wrap",
                    PlayerStage {
                        entry: active_entry.clone(), object: active_result.clone(),
                        session_id: current.playback_session, playing: current.playback_intent,
                        muted: current.muted,
                        playlist_loading: *loading.read(), playlist_loaded: *playlist_loaded.read(),
                        playlist_error: filter_error(),
                        on_handle: move |registration| controller.register_handle(registration),
                        on_event: move |event| controller.media_event(event),
                        on_retry_playlist: move |_| initial_retry_revision += 1,
                        on_edit_filter: move |_| state.write().filter_open = true,
                        on_retry_object: move |_| active_object.restart(),
                        on_skip: move |_| controller.next(),
                    }
                    if let Some(error) = current.transient_error.clone() {
                        div { class: "semantic-player__transient-error", role: "alert",
                            span { "{error}" }
                            dxcomp::Button { size: dxcomp::ButtonSize::Xs, variant: dxcomp::ButtonVariant::Outline,
                                onclick: move |_| controller.retry_current(), "Retry media" }
                            dxcomp::Button { size: dxcomp::ButtonSize::Xs, variant: dxcomp::ButtonVariant::Outline,
                                onclick: move |_| controller.next(), "Skip" }
                        }
                    }
                        }
                        if current.playlist_open {
                            PlaylistResizeHandle {}
                            PlayerPlaylist {
                                entries: current.queue.clone(), active_index: current.active_index,
                                failed_occurrences: current.failed_occurrences.clone(),
                                follow_active: *follow_active.read(),
                                on_select: move |index| controller.select(index),
                                on_remove: move |index| controller.remove(index),
                                on_move: move |(index, new_index)| controller.move_entry(index, new_index),
                                on_clear: move |_| controller.clear(),
                                on_follow_change: move |follow| follow_active.set(follow),
                                on_close: move |_| {
                                    state.write().playlist_open = false;
                                    store_playlist_open(false);
                                },
                            }
                        }
            }
            PlayerEntityDialog {
                open: current.entity_dialog_open, title: current_title,
                object: active_result, target: dialog_target,
                on_open_change: move |open| { if open { controller.open_dialog(); } else { controller.close_dialog(); } },
                on_deleted: move |_| controller.remove_active(),
                on_retry: move |_| active_object.restart(),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn load_handler(
    client: semantic_rpc::RpcClient,
    scope_id: Option<String>,
    append: bool,
    controller: PlayerController,
    mut loading: Signal<bool>,
    mut filter_error: Signal<Option<String>>,
    mut queue_warning: Signal<Option<String>>,
    mut request_generation: Signal<u64>,
    mut state: Signal<PlayerState>,
    mut playlist_loaded: Signal<bool>,
) -> EventHandler<PlaylistFilter> {
    EventHandler::new(move |filter| {
        if *loading.read() {
            return;
        }
        loading.set(true);
        filter_error.set(None);
        request_generation += 1;
        let generation = *request_generation.read();
        let client = client.clone();
        let scope_id = scope_id.clone();
        spawn(async move {
            match load_playlist(client, scope_id, filter).await {
                Ok(result) if generation == *request_generation.read() => {
                    if append {
                        controller.append(result.entries);
                    } else {
                        controller.replace(result.entries);
                    }
                    queue_warning.set(result.warning);
                    playlist_loaded.set(true);
                    state.write().filter_open = false;
                }
                Err(error) if generation == *request_generation.read() => {
                    filter_error.set(Some(error))
                }
                _ => {}
            }
            if generation == *request_generation.read() {
                loading.set(false);
            }
        });
    })
}

fn store_playlist_open(open: bool) {
    spawn(async move {
        let eval = document::eval(if open {
            "localStorage.setItem('semantic:player:playlist-open:v2', 'true');"
        } else {
            "localStorage.setItem('semantic:player:playlist-open:v2', 'false');"
        });
        let _ = eval.await;
    });
}
