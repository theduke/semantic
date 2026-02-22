#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct MapExpr {
    pub entries: Vec<MapEntryExpr>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct MapEntryExpr {
    pub key: crate::expr::Expr,
    pub value: crate::expr::Expr,
}
