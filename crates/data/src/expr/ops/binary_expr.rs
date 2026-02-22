#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct BinaryExpr {
    pub op: crate::expr::BinaryOperator,
    pub left: crate::expr::Expr,
    pub right: crate::expr::Expr,
}
