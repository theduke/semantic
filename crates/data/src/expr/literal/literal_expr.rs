#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LiteralExpr {
    #[facet(opaque)]
    pub value: crate::schema::core::literal_value::LiteralValue,
}
