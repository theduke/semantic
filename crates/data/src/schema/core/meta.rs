use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: Option<String>,
    pub description: Option<String>,

    pub id: Option<String>,
    pub deprecated: bool,

    pub aliases: Vec<String>,
    pub examples: Vec<crate::schema::core::literal_value::LiteralValue>,

    pub tags: Vec<String>,
    pub docs_url: Option<String>,

    pub annotations: BTreeMap<String, String>,
}
