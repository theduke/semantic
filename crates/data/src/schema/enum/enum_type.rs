#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct EnumType {
    pub repr: crate::schema::r#enum::enum_repr::EnumRepr,
    pub variants: Vec<crate::schema::r#enum::enum_variant::EnumVariant>,
}
