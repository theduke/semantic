use dioxus::prelude::*;

/// Consistent route-level heading, context, and actions.
///
/// Breadcrumb and action content are slots so routes can compose links and
/// domain-specific controls without expanding this component's prop surface.
#[derive(Clone, PartialEq, Props)]
pub struct PageHeaderProps {
    #[props(into)]
    pub title: String,

    #[props(default)]
    pub description: Option<String>,

    #[props(default)]
    pub breadcrumbs: Option<Element>,

    #[props(default)]
    pub actions: Option<Element>,
}

#[component]
pub fn PageHeader(props: PageHeaderProps) -> Element {
    rsx! {
        header { class: "semantic-page-header",
            if let Some(breadcrumbs) = props.breadcrumbs {
                nav { class: "semantic-page-header__breadcrumbs", aria_label: "Breadcrumb",
                    {breadcrumbs}
                }
            }
            div { class: "semantic-page-header__content",
                div { class: "semantic-page-header__copy",
                    h1 { class: "semantic-page-header__title", "{props.title}" }
                    if let Some(description) = props.description {
                        p { class: "semantic-page-header__description", "{description}" }
                    }
                }
                if let Some(actions) = props.actions {
                    div { class: "semantic-page-header__actions", {actions} }
                }
            }
        }
    }
}
