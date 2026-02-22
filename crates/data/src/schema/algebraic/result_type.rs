#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ResultType {
    pub ok: Box<crate::schema::core::type_node::Type>,
    pub err: Box<crate::schema::core::type_node::Type>,
}
