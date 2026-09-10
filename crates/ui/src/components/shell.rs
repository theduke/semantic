use dioxus::prelude::*;

use super::GlobalSearch;
use crate::views::Route;

const MAIN_CONTENT_ID: &str = "semantic-main-content";
const PRIMARY_NAV_ID: &str = "semantic-primary-navigation";

/// Controls the layout constraints applied by [`AppFrame`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppFrameVariant {
    /// The standard, document-scrolling application layout.
    #[default]
    Standard,
    /// A viewport-constrained layout for immersive tools such as the player.
    Immersive,
}

/// Shared application chrome and main-content landmark.
#[component]
pub fn AppFrame(#[props(default)] variant: AppFrameVariant, children: Element) -> Element {
    let (frame_class, main_class) = match variant {
        AppFrameVariant::Standard => ("semantic-ui", "semantic-ui__main"),
        AppFrameVariant::Immersive => ("semantic-player-shell", "semantic-player-shell__main"),
    };

    rsx! {
        div { class: frame_class,
            a { class: "semantic-skip-link", href: "#{MAIN_CONTENT_ID}", "Skip to main content" }
            AppFrameHeader {}
            main { id: MAIN_CONTENT_ID, class: main_class, tabindex: "-1", {children} }
        }
    }
}

#[component]
pub fn AppShell() -> Element {
    rsx! {
        AppFrame { Outlet::<Route> {} }
    }
}

#[component]
pub fn PlayerShell() -> Element {
    rsx! {
        AppFrame { variant: AppFrameVariant::Immersive, Outlet::<Route> {} }
    }
}

#[component]
fn AppFrameHeader() -> Element {
    rsx! {
        header { class: "semantic-ui__header",
            div { class: "semantic-ui__identity",
                Link {
                    to: Route::HomePage,
                    class: "semantic-ui__brand",
                    aria_label: "Semantic home",
                    span { aria_hidden: "true", class: "semantic-ui__brand-mark", "S" }
                    span { class: "semantic-ui__brand-name", "Semantic" }
                }
                GlobalSearch {}
            }
            PrimaryNav {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NavItem {
    Jobs,
    Home,
    Browse,
    Tree,
    Labels,
    CreateEntity,
    Upload,
    Record,
    Player,
    Data,
}

/// The application's grouped primary navigation.
///
/// Its only mutable state is whether the mobile menu is open. Route matching is
/// derived directly from the router, keeping the render path pure and ensuring
/// browser back/forward navigation updates the active item.
#[component]
pub fn PrimaryNav() -> Element {
    let route = use_route::<Route>();
    let mut menu_open = use_signal(|| false);
    let menu_state = if menu_open() { "open" } else { "closed" };

    rsx! {
        button {
            r#type: "button",
            class: "semantic-primary-nav__toggle",
            aria_controls: PRIMARY_NAV_ID,
            aria_expanded: menu_open(),
            onclick: move |_| menu_open.toggle(),
            onkeydown: move |event| {
                if event.key() == Key::Escape {
                    menu_open.set(false);
                }
            },
            span { aria_hidden: "true", "☰" }
            span { "Menu" }
        }
        nav {
            id: PRIMARY_NAV_ID,
            class: "semantic-primary-nav",
            aria_label: "Primary navigation",
            "data-state": menu_state,
            onkeydown: move |event| {
                if event.key() == Key::Escape {
                    menu_open.set(false);
                }
            },

            NavGroup { label: "Workspace",
                PrimaryNavLink {
                    to: Route::HomePage,
                    label: "Home",
                    active: nav_item_is_active(&route, NavItem::Home),
                    on_navigate: move |_| menu_open.set(false),
                }
            }
            NavGroup { label: "Explore",
                PrimaryNavLink {
                    to: Route::LabelsPage, label: "Labels",
                    active: nav_item_is_active(&route, NavItem::Labels),
                    on_navigate: move |_| menu_open.set(false),
                }
                PrimaryNavLink {
                    to: Route::BrowsePage {
                        collection: None,
                        view: None,
                        renderer: None,
                        page: None,
                        page_size: None,
                        filters: None,
                        sql: None,
                    },
                    label: "Browse",
                    active: nav_item_is_active(&route, NavItem::Browse),
                    on_navigate: move |_| menu_open.set(false),
                }
                PrimaryNavLink {
                    to: Route::TreePage { root: None, hierarchy: None, kind: None },
                    label: "Tree",
                    active: nav_item_is_active(&route, NavItem::Tree),
                    on_navigate: move |_| menu_open.set(false),
                }
            }
            NavGroup { label: "Create",
                PrimaryNavLink {
                    to: Route::CreateEntityPage,
                    label: "New entity",
                    active: nav_item_is_active(&route, NavItem::CreateEntity),
                    on_navigate: move |_| menu_open.set(false),
                }
                PrimaryNavLink {
                    to: Route::UploadPage,
                    label: "Upload",
                    active: nav_item_is_active(&route, NavItem::Upload),
                    on_navigate: move |_| menu_open.set(false),
                }
                PrimaryNavLink {
                    to: Route::RecordPage,
                    label: "Record",
                    active: nav_item_is_active(&route, NavItem::Record),
                    on_navigate: move |_| menu_open.set(false),
                }
            }
            NavGroup { label: "Tools",
                PrimaryNavLink {
                    to: Route::PlayPage,
                    label: "Player",
                    active: nav_item_is_active(&route, NavItem::Player),
                    on_navigate: move |_| menu_open.set(false),
                }
            }
            NavGroup { label: "Data tools",
                PrimaryNavLink {
                    to: Route::JobsPage,
                    label: "Jobs",
                    active: nav_item_is_active(&route, NavItem::Jobs),
                    on_navigate: move |_| menu_open.set(false),
                }
                PrimaryNavLink {
                    to: Route::DataPage,
                    label: "Data",
                    active: nav_item_is_active(&route, NavItem::Data),
                    on_navigate: move |_| menu_open.set(false),
                }
            }
        }
    }
}

#[component]
fn NavGroup(label: &'static str, children: Element) -> Element {
    rsx! {
        div { class: "semantic-primary-nav__group", role: "group", aria_label: label,
            span { class: "semantic-primary-nav__group-label", "{label}" }
            ul { class: "semantic-primary-nav__items",
                {children}
            }
        }
    }
}

#[component]
fn PrimaryNavLink(
    to: Route,
    label: &'static str,
    active: bool,
    on_navigate: EventHandler<()>,
) -> Element {
    rsx! {
        li {
            Link {
                to,
                class: "dx-button semantic-primary-nav__link",
                "data-style": "ghost",
                "data-size": "default",
                "data-active": active,
                aria_current: active.then_some("page"),
                onclick: move |_| on_navigate.call(()),
                "{label}"
            }
        }
    }
}

fn nav_item_is_active(route: &Route, item: NavItem) -> bool {
    matches!(
        (route, item),
        (Route::HomePage, NavItem::Home)
            | (Route::JobsPage, NavItem::Jobs)
            | (Route::CollectionPage { .. }, NavItem::Browse)
            | (Route::DefaultEntityPage { .. }, NavItem::Browse)
            | (Route::CollectionEntityPage { .. }, NavItem::Browse)
            | (Route::DefaultEditEntityPage { .. }, NavItem::Browse)
            | (Route::CollectionEditEntityPage { .. }, NavItem::Browse)
            | (Route::BrowsePage { .. }, NavItem::Browse)
            | (Route::TreePage { .. }, NavItem::Tree)
            | (Route::LabelsPage, NavItem::Labels)
            | (Route::CreateEntityPage, NavItem::CreateEntity)
            | (Route::UploadPage, NavItem::Upload)
            | (Route::RecordPage, NavItem::Record)
            | (Route::PlayPage, NavItem::Player)
            | (Route::DataPage, NavItem::Data)
            | (Route::CatalogPage, NavItem::Data)
            | (Route::QueryPage, NavItem::Data)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_routes_share_the_browse_navigation_item() {
        let routes = [
            Route::CollectionPage {
                collection: "main".to_string(),
            },
            Route::DefaultEntityPage {
                id: "one".to_string(),
            },
            Route::CollectionEntityPage {
                collection: "main".to_string(),
                id: "one".to_string(),
            },
            Route::DefaultEditEntityPage {
                id: "one".to_string(),
            },
            Route::CollectionEditEntityPage {
                collection: "main".to_string(),
                id: "one".to_string(),
            },
        ];

        for route in routes {
            assert!(nav_item_is_active(&route, NavItem::Browse));
            assert!(!nav_item_is_active(&route, NavItem::Home));
        }
    }

    #[test]
    fn route_specific_navigation_items_match_query_variants() {
        let browse = Route::BrowsePage {
            collection: Some("main".to_string()),
            view: Some("table".to_string()),
            renderer: None,
            page: Some(3),
            page_size: Some(25),
            filters: None,
            sql: None,
        };
        let tree = Route::TreePage {
            root: Some("folder".to_string()),
            hierarchy: Some(true),
            kind: Some("parent".to_string()),
        };

        assert!(nav_item_is_active(&browse, NavItem::Browse));
        assert!(nav_item_is_active(&tree, NavItem::Tree));
        assert!(nav_item_is_active(&Route::DataPage, NavItem::Data));
        assert!(nav_item_is_active(&Route::CatalogPage, NavItem::Data));
        assert!(nav_item_is_active(&Route::QueryPage, NavItem::Data));
        assert!(nav_item_is_active(&Route::RecordPage, NavItem::Record));
        assert_eq!(Route::RecordPage.to_string(), "/create/record");
    }
}
