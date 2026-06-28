mod browse;
mod catalog;
mod collection;
mod entity;
mod form;
mod home;
mod query;
mod tree;
mod upload;

pub use browse::BrowsePage;
pub use catalog::CatalogPage;
pub use collection::CollectionPage;
pub use entity::{CollectionEntityPage, DefaultEntityPage};
pub use form::{CollectionEditEntityPage, CreateEntityPage, DefaultEditEntityPage};
pub use home::HomePage;
pub use query::QueryPage;
pub use tree::TreePage;
pub use upload::UploadPage;

use dioxus::prelude::*;

use crate::components::AppShell;

#[derive(Clone, Debug, PartialEq, Routable)]
pub enum Route {
    #[layout(AppShell)]
    #[route("/")]
    HomePage,
    #[route("/catalog")]
    CatalogPage,
    #[route("/collections/:collection")]
    CollectionPage { collection: String },
    #[route("/entities/create")]
    CreateEntityPage,
    #[route("/entities/:id")]
    DefaultEntityPage { id: String },
    #[route("/collections/:collection/:id")]
    CollectionEntityPage { collection: String, id: String },
    #[route("/entities/:id/edit")]
    DefaultEditEntityPage { id: String },
    #[route("/collections/:collection/:id/edit")]
    CollectionEditEntityPage { collection: String, id: String },
    #[route("/browse?:collection&:view&:renderer&:page&:page_size&:sql")]
    BrowsePage {
        collection: Option<String>,
        view: Option<String>,
        renderer: Option<String>,
        page: Option<usize>,
        page_size: Option<usize>,
        sql: Option<String>,
    },
    #[route("/query")]
    QueryPage,
    #[route("/tree?:root")]
    TreePage { root: Option<String> },
    #[route("/upload")]
    UploadPage,
}
