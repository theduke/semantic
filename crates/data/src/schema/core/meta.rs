use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: Option<String>,
    pub description: Option<String>,

    pub id: Option<String>,
    pub deprecated: Option<Deprecation>,

    pub aliases: Vec<String>,
    pub examples: Vec<crate::value::Value>,

    pub tags: Vec<String>,
    pub docs_url: Option<String>,

    pub annotations: BTreeMap<String, String>,
}

#[derive(facet::Facet, Clone, Debug, Default, PartialEq)]
pub struct Deprecation {
    pub note: Option<String>,
}
