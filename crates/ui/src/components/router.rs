use brass::{
    dom::Event,
    vdom::{self, component, div, s, Render, TagBuilder},
    Str, VNode,
};
use semantic_ui_core::{routing::Route, ContextExt};

use crate::components as comps;

use super::{base::tags::TagManager, import::import_page::ImportPage};

pub fn history_push_route(route: &Route) {
    let path = route.to_path();
    let title = route.title();
    let data = wasm_bindgen::JsValue::null();
    web_sys::window()
        .unwrap()
        .history()
        .unwrap()
        .push_state_with_url(&data, title, Some(&path))
        .unwrap();
}

pub fn router(route: &Route) -> VNode {
    let nav = navbar();
    let content = match route {
        Route::Browse => component::<comps::entity::browse_page::BrowsePage>(
            comps::entity::browse_page::BrowsePageProps {},
        ),
        Route::Logout => VNode::Empty,
        Route::Import => component::<ImportPage>(()),
        Route::Upload => comps::upload::upload_page(),
        Route::Entity(ident) => component::<comps::entity::entity_page::EntityPage>(
            comps::entity::entity_page::EntityPageProps {
                ident: ident.clone(),
            },
        ),
        Route::EntityCreateSelect => {
            component::<comps::entity::entity_create_selector::EntityCreateSelectorPage>(())
        }
        Route::EntityCreate { entity_type } => {
            component::<comps::entity::entity_create_page::EntityCreatePage>(
                comps::entity::entity_create_page::EntityCreatePageProps {
                    entity_type: entity_type.clone(),
                },
            )
        }
        Route::Tags => TagManager {}.render(),
        Route::Play => {
            // Special casing for play because of overflow: hidden;
            // TODO: the router should probably just use the raw output, without
            // wrapping in in a .container below...
            let content = comps::base::play::StandalonePlayer {
                expr: None,
                keyboard_controls: true,
            }
            .render();

            return div()
                .style_raw("height: 100%; display: flex; flex-direction: column;")
                .and(nav)
                .and(
                    div()
                        .class("container")
                        .style_raw("width: 100%; flex-grow: 1; overflow: hidden;")
                        .and(content),
                )
                .build();
        }
    };

    div()
        .style_raw("height: 100%; display: flex; flex-direction: column;")
        .and(nav)
        .and(
            div()
                .class("container")
                .style_raw("width: 100%; flex-grow: 1;")
                .and(content),
        )
        .build()
}

#[derive(PartialEq, Eq)]
pub struct LinkProps {
    pub route: Route,
    pub text: Str,
    pub class: Option<Str>,
}

pub struct Link {
    props: LinkProps,
}

brass::enable_props!(LinkProps => Link);

impl brass::Component for Link {
    type Properties = LinkProps;

    type Msg = ();

    fn init(props: Self::Properties, _ctx: &mut brass::Context<Self::Msg>) -> Self {
        Self { props }
    }

    fn update(&mut self, _msg: Self::Msg, ctx: &mut brass::Context<Self::Msg>) {
        ctx.router().goto(self.props.route.clone());
    }

    fn render(&self, ctx: brass::RenderContext<Self>) -> VNode {
        vdom::a_with(self.props.text.clone())
            .class_opt(self.props.class.clone())
            .on(Event::Click, ctx.on_simple(|| ()))
            .build()
    }

    fn on_property_change(
        &mut self,
        props: Self::Properties,
        _ctx: &mut brass::Context<Self::Msg>,
    ) -> brass::ShouldRender {
        if props == self.props {
            false
        } else {
            self.props = props;
            true
        }
    }
}

fn navbar() -> TagBuilder {
    let brand = div().class("navbar-brand").and(LinkProps {
        route: Route::Browse,
        text: s("Semantic"),
        class: Some("navbar-item".into()),
    });

    let items = div()
        .class("navbar-start")
        .and(LinkProps {
            route: Route::Browse,
            text: s("Browse"),
            class: Some(s("navbar-item")),
        })
        .and(LinkProps {
            route: Route::EntityCreateSelect,
            text: s("Create"),
            class: Some(s("navbar-item")),
        })
        .and(LinkProps {
            route: Route::Upload,
            text: s("Upload"),
            class: Some(s("navbar-item")),
        })
        .and(LinkProps {
            route: Route::Import,
            text: s("Import"),
            class: Some(s("navbar-item")),
        })
        .and(LinkProps {
            route: Route::Tags,
            text: s("Tags"),
            class: Some(s("navbar-item")),
        })
        .and(LinkProps {
            route: Route::Play,
            text: s("Play"),
            class: Some(s("navbar-item")),
        });

    let logout = LinkProps {
        route: Route::Logout,
        text: "Logout".into(),
        class: Some(s("button is-light is-small")),
    };
    let actions = brass_bulma::buttons().and(logout);
    let end = div()
        .class("navbar-end")
        .and(div().class(s("navbar-item")).and(actions));

    let menu = div().class(s("navbar-menu is-active")).and((items, end));

    div().class(s("navbar")).and((brand, menu))
}
