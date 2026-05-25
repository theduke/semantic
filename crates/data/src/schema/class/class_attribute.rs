#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ClassAttribute {
    pub attribute: crate::schema::attribute::attribute_ref::AttributeRef,
    pub required: bool,
    /// If set, this attribute's value is computed from this expression
    /// at read time rather than stored. The expression must reference
    /// other attributes of the same class via `RefExpr::Identifier("self")`.
    pub computed: Option<crate::expr::Expr>,
    /// Optional extra constraints for this attribute within this class.
    pub constraints: Vec<crate::schema::constraints::constraint::Constraint>,
    pub meta: crate::schema::core::meta::Meta,
}
