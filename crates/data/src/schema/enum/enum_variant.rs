#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub value: Option<i64>,
    pub symbol: Option<String>,
    pub meta: crate::schema::core::meta::Meta,
}
