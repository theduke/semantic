use dioxus::prelude::*;

use super::GlobalSearch;
use crate::views::Route;

const MAIN_CONTENT_ID: &str = "semantic-main-content";
const MOBILE_NAV_ID: &str = "semantic-mobile-navigation";

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
            if variant == AppFrameVariant::Standard {
                div { class: "semantic-ui__layout",
                    aside { class: "semantic-ui__sidebar", aria_label: "Workspace",
                        PrimaryNav { mobile: false }
                    }
                    main { id: MAIN_CONTENT_ID, class: main_class, tabindex: "-1", {children} }
                }
            } else {
                main { id: MAIN_CONTENT_ID, class: main_class, tabindex: "-1", {children} }
            }
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
            div { class: "semantic-ui__header-actions",
                NewMenu {}
                PrimaryNav { mobile: true }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NavItem {
    Jobs,
    Import,
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

/// Primary navigation is always visible in the desktop sidebar and opens as a
/// drawer from the header on narrow screens and immersive pages.
///
/// Its only mutable state is whether the mobile menu is open. Route matching is
/// derived directly from the router, keeping the render path pure and ensuring
/// browser back/forward navigation updates the active item.
#[component]
pub fn PrimaryNav(#[props(default)] mobile: bool) -> Element {
    let route = use_route::<Route>();
    let mut menu_open = use_signal(|| false);
    let mut toggle = use_signal(|| None::<std::rc::Rc<MountedData>>);

    rsx! {
        if mobile {
            button {
                r#type: "button",
                class: "semantic-primary-nav__toggle",
                aria_controls: MOBILE_NAV_ID,
                aria_expanded: menu_open(),
                aria_label: if menu_open() { "Close navigation" } else { "Open navigation" },
                onmounted: move |event| toggle.set(Some(event.data())),
                onclick: move |_| menu_open.toggle(),
                span { aria_hidden: "true", if menu_open() { "×" } else { "☰" } }
                span { "Menu" }
            }
            if menu_open() {
                button {
                    class: "semantic-primary-nav__scrim",
                    r#type: "button",
                    aria_label: "Close navigation",
                    onclick: move |_| {
                        menu_open.set(false);
                        restore_focus(toggle);
                    },
                }
            }
        }
        if !mobile || menu_open() {
            nav {
                id: if mobile { MOBILE_NAV_ID } else { "semantic-sidebar-navigation" },
                class: if mobile { "semantic-primary-nav semantic-primary-nav--drawer" } else { "semantic-primary-nav semantic-primary-nav--sidebar" },
                aria_label: "Primary navigation",
                tabindex: if mobile { Some("-1") } else { None },
                onmounted: move |event| async move {
                    if mobile {
                        let _ = event.set_focus(true).await;
                    }
                },
                onkeydown: move |event| {
                    if mobile && event.key() == Key::Escape {
                        event.prevent_default();
                        event.stop_propagation();
                        menu_open.set(false);
                        restore_focus(toggle);
                    }
                },
                NavGroup { label: "Workspace",
                    PrimaryNavLink {
                        to: Route::HomePage,
                        label: "Home",
                        active: nav_item_is_active(&route, NavItem::Home),
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
                PrimaryNavLink {
                    to: Route::LabelsPage,
                    label: "Labels",
                    active: nav_item_is_active(&route, NavItem::Labels),
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
}

#[component]
fn NewMenu() -> Element {
    let route = use_route::<Route>();
    let navigator = use_navigator();
    let active = [
        NavItem::CreateEntity,
        NavItem::Upload,
        NavItem::Record,
        NavItem::Import,
    ]
    .into_iter()
    .any(|item| nav_item_is_active(&route, item));

    rsx! {
        dxcomp::DropdownMenu { class: "semantic-new-menu",
            dxcomp::DropdownMenuTrigger {
                class: "semantic-new-menu__trigger",
                "data-active": active,
                aria_haspopup: "menu",
                span { aria_hidden: "true", "+" }
                "New"
            }
            dxcomp::DropdownMenuContent {
                id: "semantic-new-menu-options",
                class: "semantic-new-menu__options",
                role: "menu",
                aria_label: "Create new",
                for (index, (label, target)) in [
                    ("Note", Route::CreateNotePage),
                    ("Entity", Route::CreateEntityPage),
                    ("Upload files", Route::UploadPage),
                    ("Record media", Route::RecordPage),
                    ("Import URL", Route::ImportPage),
                ].into_iter().enumerate() {
                    dxcomp::DropdownMenuItem::<Route> {
                        value: target,
                        index,
                        role: "menuitem",
                        on_select: move |target: Route| {
                            navigator.push(target);
                        },
                        "{label}"
                    }
                }
            }
        }
    }
}

fn restore_focus(element: Signal<Option<std::rc::Rc<MountedData>>>) {
    spawn(async move {
        if let Some(element) = element.peek().clone() {
            let _ = element.set_focus(true).await;
        }
    });
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
            | (Route::ImportPage, NavItem::Import)
            | (Route::CollectionPage { .. }, NavItem::Browse)
            | (Route::DefaultEntityPage { .. }, NavItem::Browse)
            | (Route::CollectionEntityPage { .. }, NavItem::Browse)
            | (Route::DefaultEditEntityPage { .. }, NavItem::Browse)
            | (Route::CollectionEditEntityPage { .. }, NavItem::Browse)
            | (Route::BrowsePage { .. }, NavItem::Browse)
            | (Route::TreePage { .. }, NavItem::Tree)
            | (Route::LabelsPage, NavItem::Labels)
            | (Route::CreateEntityPage, NavItem::CreateEntity)
            | (Route::CreateNotePage, NavItem::CreateEntity)
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
        assert!(nav_item_is_active(
            &Route::CreateNotePage,
            NavItem::CreateEntity
        ));
        assert_eq!(Route::CreateNotePage.to_string(), "/notes/create");
        assert_eq!(Route::RecordPage.to_string(), "/create/record");
    }
}
