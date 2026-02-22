#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct UnaryExpr {
    pub op: crate::expr::UnaryOperator,
    pub operand: crate::expr::Expr,
}
