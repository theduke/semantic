use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerShortcut {
    Previous,
    Next,
    TogglePlay,
    ToggleMute,
    ToggleFullscreen,
    Escape,
}

pub fn use_player_browser_bridge(
    enabled: bool,
    on_shortcut: EventHandler<PlayerShortcut>,
    on_fullscreen_change: EventHandler<bool>,
) {
    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                r#"
                const ignored = (target) => target && (
                    ['INPUT', 'TEXTAREA', 'SELECT', 'BUTTON'].includes(target.tagName) ||
                    target.isContentEditable || target.getAttribute?.('role') === 'slider'
                );
                window.__semanticPlayerKeyHandler = (event) => {
                    if (ignored(event.target) || event.ctrlKey || event.altKey || event.metaKey) return;
                    const keys = { ArrowLeft: 'previous', ArrowRight: 'next', ' ': 'toggle-play',
                        m: 'toggle-mute', M: 'toggle-mute', f: 'toggle-fullscreen', F: 'toggle-fullscreen', Escape: 'escape' };
                    const action = keys[event.key]; if (!action) return;
                    if (['ArrowLeft', 'ArrowRight', ' '].includes(event.key)) event.preventDefault();
                    dioxus.send(action);
                };
                window.__semanticPlayerFullscreenHandler = () => dioxus.send(document.fullscreenElement ? 'fullscreen-on' : 'fullscreen-off');
                window.addEventListener('keydown', window.__semanticPlayerKeyHandler);
                document.addEventListener('fullscreenchange', window.__semanticPlayerFullscreenHandler);
            "#,
            );
            while let Ok(action) = eval.recv::<String>().await {
                match action.as_str() {
                    "fullscreen-on" => on_fullscreen_change.call(true),
                    "fullscreen-off" => on_fullscreen_change.call(false),
                    _ if !enabled => {}
                    "previous" => on_shortcut.call(PlayerShortcut::Previous),
                    "next" => on_shortcut.call(PlayerShortcut::Next),
                    "toggle-play" => on_shortcut.call(PlayerShortcut::TogglePlay),
                    "toggle-mute" => on_shortcut.call(PlayerShortcut::ToggleMute),
                    "toggle-fullscreen" => on_shortcut.call(PlayerShortcut::ToggleFullscreen),
                    "escape" => on_shortcut.call(PlayerShortcut::Escape),
                    _ => {}
                }
            }
        });
    });
    use_drop(|| {
        _ = document::eval(
            r#"
            window.removeEventListener('keydown', window.__semanticPlayerKeyHandler);
            document.removeEventListener('fullscreenchange', window.__semanticPlayerFullscreenHandler);
            delete window.__semanticPlayerKeyHandler; delete window.__semanticPlayerFullscreenHandler;
        "#,
        );
    });
}

pub fn toggle_fullscreen() {
    _ = document::eval(
        r#"
        const root = document.getElementById('semantic-player-root');
        if (document.fullscreenElement) document.exitFullscreen();
        else if (root && root.requestFullscreen) root.requestFullscreen();
    "#,
    );
}
