#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum LikeKind {
    Like,
    SimilarTo,
}

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
    Subquery(Box<crate::expr::Select>),
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct LikeExpr {
    pub kind: LikeKind,
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
    pub query: Box<crate::expr::Select>,
    pub negated: bool,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct RegexExpr {
    pub value: crate::expr::Expr,
    pub pattern: crate::expr::Expr,
    pub case_insensitive: bool,
    pub negated: bool,
}
