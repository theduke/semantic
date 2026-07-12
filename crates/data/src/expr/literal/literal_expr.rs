#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LiteralExpr {
    pub value: crate::value::Value,
}
