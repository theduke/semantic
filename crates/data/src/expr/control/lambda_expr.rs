#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct LambdaExpr {
    pub params: Vec<LambdaParam>,
    pub body: crate::expr::Expr,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct LambdaParam {
    pub name: String,
    #[facet(opaque)]
    pub ty: Option<crate::schema::core::type_node::Type>,
}
