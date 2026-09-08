mod audio_recording;
mod browse;
mod catalog;
mod collection;
mod data;
mod entity;
mod form;
mod home;
mod play;
mod query;
mod tree;
mod upload;

pub use audio_recording::AudioRecordingPage;
pub use browse::BrowsePage;
pub use catalog::CatalogPage;
pub use collection::CollectionPage;
pub use data::DataPage;
pub use entity::{CollectionEntityPage, DefaultEntityPage};
pub use form::{CollectionEditEntityPage, CreateEntityPage, DefaultEditEntityPage};
pub use home::HomePage;
pub use play::PlayPage;
pub use query::QueryPage;
pub use tree::TreePage;
pub use upload::UploadPage;

use dioxus::prelude::*;

use crate::components::{AppShell, PlayerShell};

#[derive(Clone, Debug, PartialEq, Routable)]
pub enum Route {
    #[layout(AppShell)]
    #[route("/")]
    HomePage,
    #[route("/data")]
    DataPage,
    #[route("/data/catalog")]
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
    #[route("/browse?:collection&:view&:renderer&:page&:page_size&:filters&:sql")]
    BrowsePage {
        collection: Option<String>,
        view: Option<String>,
        renderer: Option<String>,
        page: Option<usize>,
        page_size: Option<usize>,
        filters: Option<String>,
        sql: Option<String>,
    },
    #[route("/data/query")]
    QueryPage,
    #[route("/tree?:root&:hierarchy&:kind")]
    TreePage {
        root: Option<String>,
        hierarchy: Option<bool>,
        kind: Option<String>,
    },
    #[route("/upload")]
    UploadPage,
    #[route("/create/audio-recording")]
    AudioRecordingPage,
    #[end_layout]
    #[layout(PlayerShell)]
    #[route("/play")]
    PlayPage,
}
