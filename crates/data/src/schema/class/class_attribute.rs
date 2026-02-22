#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ClassAttribute {
    pub attribute: crate::schema::attribute::attribute_ref::AttributeRef,
    pub required: bool,
    /// Optional extra constraints for this attribute within this class.
    pub constraints: Vec<crate::schema::constraints::constraint::Constraint>,
    pub meta: crate::schema::core::meta::Meta,
}
