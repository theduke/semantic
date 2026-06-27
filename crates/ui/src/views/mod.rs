mod catalog;
mod collection;
mod entity;
mod form;
mod home;
mod query;

pub use catalog::CatalogPage;
pub use collection::CollectionPage;
pub use entity::EntityPage;
pub use form::{CreateEntityPage, EditEntityPage};
pub use home::HomePage;
pub use query::QueryPage;

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
    #[route("/collections/:collection/:id")]
    EntityPage { collection: String, id: String },
    #[route("/collections/:collection/:id/edit")]
    EditEntityPage { collection: String, id: String },
    #[route("/query")]
    QueryPage,
}
