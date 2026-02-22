use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum LiteralValue {
    Null,
    Bool(bool),
    Int(i128),
    UInt(u128),
    Float(String),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<LiteralValue>),
    Map(BTreeMap<String, LiteralValue>),
}
