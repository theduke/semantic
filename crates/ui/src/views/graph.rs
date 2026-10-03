use crate::{components::PageHeader, views::Route};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use dioxus::prelude::*;
use dxgraph::{LayoutConfig, use_graph_controller};
use semantic_ui_core::{
    EntityAutocomplete, EntityTarget,
    graph::{EntityGraphView, GraphMode},
};
use std::collections::HashSet;

pub(crate) fn graph_layout(mode: GraphMode, value: Option<&str>) -> LayoutConfig {
    match value {
        Some("tree") => LayoutConfig::Tree(Default::default()),
        Some("mindmap") => LayoutConfig::MindMap(Default::default()),
        Some("radial") => LayoutConfig::Radial(Default::default()),
        Some("force") => LayoutConfig::Force(Default::default()),
        _ if mode == GraphMode::Hierarchy => LayoutConfig::Tree(Default::default()),
        _ => LayoutConfig::Force(Default::default()),
    }
}
fn layout_name(layout: &LayoutConfig) -> &'static str {
    match layout {
        LayoutConfig::Tree(_) => "tree",
        LayoutConfig::MindMap(_) => "mindmap",
        LayoutConfig::Radial(_) => "radial",
        _ => "force",
    }
}
fn graph_route(root: Option<String>, mode: GraphMode, layout: Option<String>) -> Route {
    Route::GraphPage {
        root: root
            .filter(|id| !id.trim().is_empty())
            .map(encode_graph_param),
        mode: Some(mode.as_str().into()),
        layout,
    }
}

// Dioxus decodes the full query before splitting its arguments, so reserved
// characters inside ids need an unambiguous, delimiter-free representation.
fn encode_graph_param(value: String) -> String {
    if value.starts_with("b64:")
        || value
            .chars()
            .any(|ch| !ch.is_ascii_alphanumeric() && !matches!(ch, '-' | '_' | '.' | ':'))
    {
        format!("b64:{}", URL_SAFE_NO_PAD.encode(value))
    } else {
        value
    }
}
fn decode_graph_param(value: String) -> String {
    value
        .strip_prefix("b64:")
        .and_then(|encoded| URL_SAFE_NO_PAD.decode(encoded).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or(value)
}

#[component]
pub fn GraphPage(root: Option<String>, mode: Option<String>, layout: Option<String>) -> Element {
    let root = root.map(decode_graph_param);
    let mode = GraphMode::parse(mode.as_deref());
    let layout_config = graph_layout(mode, layout.as_deref());
    let controller = use_graph_controller();
    let nav = navigator();
    let picker_layout = layout.clone();
    let mode_root = root.clone();
    let layout_root = root.clone();
    let selected_mode: usize = match mode {
        GraphMode::Hierarchy => 0,
        GraphMode::Relations => 1,
        GraphMode::Both => 2,
    };
    let selected_layout = use_memo(use_reactive((&layout_config,), |(layout,)| {
        Some(layout_name(&layout).to_owned())
    }));
    let focus_layout = layout.clone();
    rsx! {
        div { class: "semantic-page semantic-graph-page",
            PageHeader {
                title: "Graph",
                description: Some("Explore entity hierarchies and relationships.".into()),
            }
            div { class: "semantic-graph-toolbar", aria_label: "Graph options",
                EntityAutocomplete {
                    value: root.clone(),
                    search_fields: Some(vec!["id".into(), "semantic:title".into()]),
                    aria_label: "Graph root",
                    placeholder: "Choose a root entity",
                    on_value_change: move |root| {
                        nav.replace(
                            graph_route(root, mode, picker_layout.clone()),
                        );
                    },
                }
                dxcomp::toggle_group::ToggleGroup {
                    pressed: Some(HashSet::from([selected_mode])),
                    horizontal: true,
                    on_pressed_change: move |items: HashSet<usize>| {
                        if let Some(index) = items.iter().next() {
                            let mode = match index {
                                1 => GraphMode::Relations,
                                2 => GraphMode::Both,
                                _ => GraphMode::Hierarchy,
                            };
                            nav.replace(
                                graph_route(mode_root.clone(), mode, None),
                            );
                        }
                    },
                    dxcomp::toggle_group::ToggleItem { index: 0usize, "Hierarchy" }
                    dxcomp::toggle_group::ToggleItem { index: 1usize, "Relations" }
                    dxcomp::toggle_group::ToggleItem { index: 2usize, "Both" }
                }
                dxcomp::select::Select::<String> {
                    value: Some(selected_layout.into()),
                    aria_label: "Graph layout",
                    on_value_change: move |layout| {
                        nav.replace(
                            graph_route(layout_root.clone(), mode, layout),
                        );
                    },
                    for (index, (value, label)) in [("tree", "Tree"), ("mindmap", "Mind map"), ("force", "Force"), ("radial", "Radial")]
                        .into_iter()
                        .enumerate()
                    {
                        dxcomp::select::SelectOption::<String> {
                            value: value.to_owned(),
                            index,
                            text_value: label.to_owned(),
                            "{label}"
                        }
                    }
                }
                dxcomp::button::Button { onclick: move |_| controller.relayout(), "Re-layout" }
                dxcomp::button::Button {
                    onclick: move |_| controller.reset_positions(),
                    "Reset positions"
                }
                dxcomp::button::Button { onclick: move |_| controller.fit_view(), "Fit" }
                Link {
                    to: Route::BrowsePage {
                        collection: None,
                        view: Some("table".into()),
                        renderer: None,
                        page: None,
                        page_size: None,
                        filters: None,
                        sql: None,
                    },
                    "Browse as a list"
                }
            }
            if let Some(root) = root.filter(|id| !id.trim().is_empty()) {
                EntityGraphView {
                    root,
                    mode,
                    layout: layout_config,
                    controller,
                    on_focus: move |target: EntityTarget| {
                        nav.replace(
                            graph_route(Some(target.id), mode, focus_layout.clone()),
                        );
                    },
                }
            } else {
                div { class: "semantic-empty-state", role: "status",
                    p { "Choose an entity above to explore its hierarchy and relationships." }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parameters_have_bounded_defaults_and_layouts() {
        assert_eq!(GraphMode::parse(Some("invalid")), GraphMode::Hierarchy);
        assert!(matches!(
            graph_layout(GraphMode::Relations, None),
            LayoutConfig::Force(_)
        ));
        assert!(matches!(
            graph_layout(GraphMode::Hierarchy, None),
            LayoutConfig::Tree(_)
        ));
        for name in ["tree", "mindmap", "force", "radial"] {
            assert_eq!(
                layout_name(&graph_layout(GraphMode::Both, Some(name))),
                name
            );
        }
        assert_eq!(
            graph_route(Some(" ".into()), GraphMode::Hierarchy, None),
            Route::GraphPage {
                root: None,
                mode: Some("hierarchy".into()),
                layout: None
            }
        );
    }
    #[test]
    fn graph_route_urls_round_trip_encoded_query_parameters() {
        let route = graph_route(
            Some("root / &?#".into()),
            GraphMode::Both,
            Some("radial".into()),
        );
        let url = route.to_string();
        assert!(url.starts_with("/graph?"));
        assert!(!url.contains("collection="));
        assert_eq!(url.parse::<Route>().unwrap(), route);
        let Route::GraphPage { root, .. } = route else {
            panic!("graph route")
        };
        assert_eq!(root.map(decode_graph_param).as_deref(), Some("root / &?#"));
    }
    #[test]
    fn legacy_collection_parameters_are_ignored_and_not_emitted() {
        let route = "/graph?root=root&collection=custom&mode=relations&layout=radial"
            .parse::<Route>()
            .unwrap();
        assert_eq!(
            route,
            graph_route(
                Some("root".into()),
                GraphMode::Relations,
                Some("radial".into())
            )
        );
        assert!(!route.to_string().contains("collection="));
    }
}
