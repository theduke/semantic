#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct CaseExpr {
    pub operand: Option<crate::expr::Expr>,
    pub branches: Vec<CaseBranch>,
    pub else_expr: Option<crate::expr::Expr>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct CaseBranch {
    pub when: crate::expr::Expr,
    pub then_expr: crate::expr::Expr,
}
