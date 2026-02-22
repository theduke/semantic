use super::Value;

#[derive(facet::Facet, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[facet(transparent)]
pub struct Map(std::collections::BTreeMap<Value, Value>);
