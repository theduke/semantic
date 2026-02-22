#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct FieldAccessExpr {
    pub target: crate::expr::Expr,
    pub field: String,
}
