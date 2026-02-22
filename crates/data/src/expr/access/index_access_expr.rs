#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct IndexAccessExpr {
    pub target: crate::expr::Expr,
    pub index: crate::expr::Expr,
}
