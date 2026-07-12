#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct Field {
    pub ty: crate::schema::core::type_node::Type,
    pub required: bool,
    pub readonly: bool,
    pub writeonly: bool,
    pub default: Option<crate::value::Value>,
    pub meta: crate::schema::core::meta::Meta,
}
