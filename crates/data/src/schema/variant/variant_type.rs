#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct VariantType {
    pub tag: crate::schema::variant::variant_tag::VariantTag,
    pub variants: Vec<crate::schema::variant::variant_case::VariantCase>,
}
