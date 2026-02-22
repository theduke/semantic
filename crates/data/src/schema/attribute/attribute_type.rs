#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct AttributeType {
    /// Globally unique attribute identifier.
    pub id: String,
    /// Human-readable attribute name.
    pub name: String,
    /// The value type carried by this attribute.
    pub ty: crate::schema::core::type_node::Type,
    /// Optional additional constraints specific to this attribute.
    pub constraints: Vec<crate::schema::constraints::constraint::Constraint>,
    pub meta: crate::schema::core::meta::Meta,
}
