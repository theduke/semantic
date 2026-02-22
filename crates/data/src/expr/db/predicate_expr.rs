#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct BetweenExpr {
    pub value: crate::expr::Expr,
    pub lower: crate::expr::Expr,
    pub upper: crate::expr::Expr,
    pub negated: bool,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct InExpr {
    pub value: crate::expr::Expr,
    pub set: InSet,
    pub negated: bool,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum InSet {
    Exprs(Vec<crate::expr::Expr>),
    Subquery(Box<crate::expr::SelectQuery>),
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct LikeExpr {
    pub value: crate::expr::Expr,
    pub pattern: crate::expr::Expr,
    pub escape: Option<crate::expr::Expr>,
    pub case_insensitive: bool,
    pub negated: bool,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct IsNullExpr {
    pub value: crate::expr::Expr,
    pub negated: bool,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ExistsExpr {
    pub query: Box<crate::expr::SelectQuery>,
    pub negated: bool,
}
