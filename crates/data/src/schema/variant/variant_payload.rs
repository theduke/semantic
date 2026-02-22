#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum VariantPayload {
    Unit,
    Tuple(Vec<crate::schema::core::type_node::Type>),
    Record(crate::schema::record::record_type::RecordType),
    Newtype(Box<crate::schema::core::type_node::Type>),
}
