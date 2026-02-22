#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct IfExpr {
    pub condition: crate::expr::Expr,
    pub then_expr: crate::expr::Expr,
    pub else_expr: crate::expr::Expr,
}
