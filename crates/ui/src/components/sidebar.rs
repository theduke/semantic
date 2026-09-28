//! Collapsible desktop sidebar state.
//!
//! The preference is shared across layouts through a global signal so that
//! switching between the standard and immersive shells does not reload it,
//! and it is persisted in local storage so it survives reloads.

use dioxus::prelude::*;
use dioxus_icons::lucide::{PanelLeftClose, PanelLeftOpen};

const STORAGE_KEY: &str = "semantic:sidebar:collapsed:v1";
pub(super) const SIDEBAR_NAV_ID: &str = "semantic-sidebar-navigation";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct SidebarState {
    /// Whether the desktop sidebar is reduced to an icon rail.
    pub collapsed: bool,
    /// Whether the stored preference has been requested from local storage.
    restore_requested: bool,
    /// Set once the user toggles the sidebar, so restoring the stored
    /// preference on load applies instantly instead of animating.
    pub animate: bool,
}

impl SidebarState {
    pub fn data_state(self) -> &'static str {
        if self.collapsed {
            "collapsed"
        } else {
            "expanded"
        }
    }
}

static SIDEBAR: GlobalSignal<SidebarState> = Signal::global(SidebarState::default);

/// Returns the shared sidebar state, restoring the persisted preference on
/// first use.
pub(super) fn use_sidebar_state() -> SidebarState {
    use_hook(|| {
        if !SIDEBAR.peek().restore_requested {
            SIDEBAR.write().restore_requested = true;
            spawn(restore_collapsed());
        }
    });
    SIDEBAR()
}

fn toggle_collapsed() {
    let collapsed = {
        let mut state = SIDEBAR.write();
        state.collapsed = !state.collapsed;
        state.animate = true;
        state.collapsed
    };
    store_collapsed(collapsed);
}

async fn restore_collapsed() {
    let eval = document::eval(&format!(
        "try {{ return localStorage.getItem('{STORAGE_KEY}'); }} catch (_) {{ return null; }}"
    ));
    if let Ok(value) = eval.await
        && let Some(collapsed) = parse_stored_collapsed(value.as_str())
    {
        let mut state = SIDEBAR.write();
        // A toggle issued before the preference loaded takes precedence.
        if !state.animate {
            state.collapsed = collapsed;
        }
    }
}

fn store_collapsed(collapsed: bool) {
    spawn(async move {
        let eval = document::eval(&format!(
            "try {{ localStorage.setItem('{STORAGE_KEY}', '{collapsed}'); }} catch (_) {{}}"
        ));
        let _ = eval.await;
    });
}

fn parse_stored_collapsed(value: Option<&str>) -> Option<bool> {
    match value? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Collapses or expands the desktop sidebar.
///
/// Also registers the `Ctrl+\` / `Cmd+\` shortcut, which only applies while
/// the desktop sidebar layout is active.
#[component]
pub(super) fn SidebarToggle() -> Element {
    let state = use_sidebar_state();
    use_sidebar_shortcut();

    let (label, tooltip) = if state.collapsed {
        ("Expand sidebar", "Expand sidebar (Ctrl+\\)")
    } else {
        ("Collapse sidebar", "Collapse sidebar (Ctrl+\\)")
    };

    rsx! {
        button {
            r#type: "button",
            class: "semantic-sidebar-toggle",
            aria_label: label,
            aria_controls: SIDEBAR_NAV_ID,
            aria_expanded: !state.collapsed,
            aria_keyshortcuts: "Control+Backslash Meta+Backslash",
            "data-tooltip": tooltip,
            onclick: move |_| toggle_collapsed(),
            if state.collapsed {
                PanelLeftOpen { width: 18, height: 18 }
            } else {
                PanelLeftClose { width: 18, height: 18 }
            }
        }
    }
}

fn use_sidebar_shortcut() {
    use_effect(move || {
        spawn(async move {
            let mut eval = document::eval(
                r#"
                if (window.__semanticSidebarKeyHandler) {
                    window.removeEventListener('keydown', window.__semanticSidebarKeyHandler);
                }
                window.__semanticSidebarKeyHandler = (event) => {
                    if (event.defaultPrevented || event.repeat || event.isComposing || !(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey || event.key !== '\\') return;
                    if (!window.matchMedia('(min-width: 861px)').matches) return;
                    event.preventDefault();
                    dioxus.send(true);
                };
                window.addEventListener('keydown', window.__semanticSidebarKeyHandler);
                "#,
            );
            while eval.recv::<bool>().await.is_ok() {
                toggle_collapsed();
            }
        });
    });
    use_drop(|| {
        _ = document::eval(
            r#"
            window.removeEventListener('keydown', window.__semanticSidebarKeyHandler);
            delete window.__semanticSidebarKeyHandler;
            "#,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_preference_parses_only_known_values() {
        assert_eq!(parse_stored_collapsed(Some("true")), Some(true));
        assert_eq!(parse_stored_collapsed(Some("false")), Some(false));
        assert_eq!(parse_stored_collapsed(Some("yes")), None);
        assert_eq!(parse_stored_collapsed(None), None);
    }
}
