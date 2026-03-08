#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum CompareOp {
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Concat,
    And,
    Or,
    Eq,
    NotEq,
    Lt,
    Lte,
    Gt,
    Gte,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum PatternMatchKind {
    Like,
    SimilarTo,
}

#[derive(facet::Facet, Debug, Clone, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum FunctionArg<T> {
    Expr(T),
    Wildcard,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Full,
}
