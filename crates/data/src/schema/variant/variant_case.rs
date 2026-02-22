#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct VariantCase {
    pub name: String,
    pub payload: crate::schema::variant::variant_payload::VariantPayload,
    pub discriminant: Option<crate::schema::core::literal_value::LiteralValue>,
    pub meta: crate::schema::core::meta::Meta,
}
