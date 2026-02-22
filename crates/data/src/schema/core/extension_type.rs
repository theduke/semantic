use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ExtensionType {
    pub namespace: String,
    pub name: String,
    pub payload: BTreeMap<String, String>,
}
