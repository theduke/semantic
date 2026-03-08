#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Expr {
    Literal(crate::expr::LiteralExpr),
    Ref(crate::expr::RefExpr),

    Unary(Box<crate::expr::UnaryExpr>),
    Binary(Box<crate::expr::BinaryExpr>),

    Call(Box<crate::expr::CallExpr>),

    FieldAccess(Box<crate::expr::FieldAccessExpr>),
    IndexAccess(Box<crate::expr::IndexAccessExpr>),

    Tuple(crate::expr::TupleExpr),
    List(crate::expr::ListExpr),
    Map(crate::expr::MapExpr),

    Cast(Box<crate::expr::CastExpr>),

    If(Box<crate::expr::IfExpr>),
    Case(Box<crate::expr::CaseExpr>),
    Let(Box<crate::expr::LetExpr>),
    Lambda(Box<crate::expr::LambdaExpr>),

    Between(Box<crate::expr::BetweenExpr>),
    In(Box<crate::expr::InExpr>),
    Like(Box<crate::expr::LikeExpr>),
    Regex(Box<crate::expr::RegexExpr>),
    IsNull(Box<crate::expr::IsNullExpr>),
    Exists(Box<crate::expr::ExistsExpr>),

    Query(Box<crate::expr::Query>),
    Subquery(Box<crate::expr::SubqueryExpr>),
}
