#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct CastExpr {
    pub expr: crate::expr::Expr,
    #[facet(opaque)]
    pub to: crate::schema::core::type_node::Type,
    pub safe: bool,
}
