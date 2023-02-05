use factdb::DataMap;
use url::Url;

pub const TY_LISTING: &str = "semantic/Listing";

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct Listing {
    #[serde(rename = "semantic/title")]
    pub title: Option<String>,

    #[serde(rename = "semantic/description")]
    pub description: Option<String>,

    #[serde(rename = "semantic/children")]
    pub children: Vec<DataMap>,

    pub load_more_link: Option<ListingLink>,

    pub related_links: Vec<ListingLink>,

    #[serde(rename = "semantic/related_items")]
    pub related_items: Vec<DataMap>,

    #[serde(flatten)]
    pub extra: DataMap,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ListingLink {
    #[serde(rename = "semantic/title")]
    pub title: Option<String>,

    #[serde(rename = "semantic/url")]
    pub url: Option<Url>,

    #[serde(rename = "semantic/preview_image")]
    pub preview_image: Option<Url>,

    #[serde(flatten)]
    pub extra: DataMap,
}
