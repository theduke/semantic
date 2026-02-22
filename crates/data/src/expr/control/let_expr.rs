#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct LetExpr {
    pub bindings: Vec<LetBinding>,
    pub body: crate::expr::Expr,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct LetBinding {
    pub name: String,
    pub value: crate::expr::Expr,
}
