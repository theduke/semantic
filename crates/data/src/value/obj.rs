use std::collections::BTreeMap;

use super::Value;

#[derive(facet::Facet, Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[facet(transparent)]
pub struct Object(BTreeMap<String, Value>);

impl Object {
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn into_btree(self) -> BTreeMap<String, Value> {
        self.0
    }
}
