#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum ClassConstraint {
    /// Constraint applied to a single class attribute.
    Field {
        attribute: crate::schema::attribute::attribute_ref::AttributeRef,
        constraint: crate::schema::constraints::constraint::Constraint,
    },
    /// Multi-field constraint placeholder. Expression semantics can be added later.
    MultiFieldExpr {
        expr: crate::expr::Expr,
        description: Option<String>,
    },
}
