use brass::{
    dom::{Attr, Event},
    vdom::{self, component, div, TagBuilder},
    Callback, VNode,
};

use crate::components as comps;

use super::{import::import_page::ImportPage, ContextExt};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Browse,
    Import,
    Entity(factordb::Ident),
}

pub struct Router {
    callback: Callback<Route>,
}

impl Router {
    pub fn new(callback: Callback<Route>) -> Self {
        Self { callback }
    }

    pub fn goto(&self, route: Route) {
        self.callback.send(route);
    }
}

pub fn router(route: &Route) -> VNode {
    let nav = navbar();
    let content = match route {
        Route::Browse => component::<comps::entity::browse_page::BrowsePage>(
            comps::entity::browse_page::BrowsePageProps {},
        ),
        Route::Import => component::<ImportPage>(()),
        Route::Entity(ident) => component::<comps::entity::entity_page::EntityPage>(
            comps::entity::entity_page::EntityPageProps {
                ident: ident.clone(),
            },
        ),
    };

    div()
        .and(nav)
        .and(div().class("container").and(content))
        .build()
}

#[derive(PartialEq, Eq)]
pub struct LinkProps {
    pub route: Route,
    pub text: String,
    pub class: Option<String>,
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
        vdom::a_with(&self.props.text)
            .class_opt(self.props.class.as_ref())
            .on(Event::Click, ctx.callback(|_: web_sys::Event| ()))
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
    let brand = div().class("navbar-brand").and(
        vdom::a()
            .class("navbar-item")
            .attr(Attr::Href, "/")
            .and("Semantic"),
    );
    let menu = div().class("navbar-menu is-active").and(
        div()
            .class("navbar-start")
            .and(LinkProps {
                route: Route::Browse,
                text: "Browse".to_string(),
                class: Some("navbar-item".to_string()),
            })
            .and(LinkProps {
                route: Route::Import,
                text: "Import".to_string(),
                class: Some("navbar-item".to_string()),
            }),
    );

    let end = div().class("navbar-brand");

    div().class("navbar").and(brand).and(menu).and(end)
    // <nav className="navbar" role="navigation" aria-label="main navigation">
    //   <div className="navbar-brand">
    //     <a className="navbar-item" href="https://bulma.io">
    //       <img
    //         src="https://bulma.io/images/bulma-logo.png"
    //         width="112"
    //         height="28"
    //       />
    //     </a>

    //     <a
    //       role="button"
    //       className="navbar-burger"
    //       aria-label="menu"
    //       aria-expanded="false"
    //       data-target="navbarBasicExample"
    //     >
    //       <span aria-hidden="true"></span>
    //       <span aria-hidden="true"></span>
    //       <span aria-hidden="true"></span>
    //     </a>
    //   </div>

    //   <div id="navbarBasicExample" className="navbar-menu is-active">
    //     <div className="navbar-start">
    //       <Link to="/" className="navbar-item">
    //         Nodes
    //       </Link>

    //       <Link to="/create" className="navbar-item">
    //         Create
    //       </Link>

    //       <Link to="/import" className="navbar-item">
    //         Import
    //       </Link>

    //       {/* <a className="navbar-item">
    //       Documentation
    //     </a>

    //     <div className="navbar-item has-dropdown is-hoverable">
    //       <a className="navbar-link">
    //         More
    //       </a>

    //       <div className="navbar-dropdown">
    //         <a className="navbar-item">
    //           About
    //         </a>
    //         <a className="navbar-item">
    //           Jobs
    //         </a>
    //         <a className="navbar-item">
    //           Contact
    //         </a>
    //         <hr className="navbar-divider" />
    //         <a className="navbar-item">
    //           Report an issue
    //         </a>
    //       </div>
    //     </div> */}
    //     </div>

    //     <div className="navbar-end">
    //       {/* <div className="navbar-item">
    //       <div className="buttons">
    //         <a className="button is-primary">
    //           <strong>Sign up</strong>
    //         </a>
    //         <a className="button is-light">
    //           Log in
    //         </a>
    //       </div>
    //     </div> */}
    //     </div>
    //   </div>
    // </nav>
}
