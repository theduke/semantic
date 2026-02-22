use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct RecordType {
    pub fields:
        BTreeMap<crate::schema::record::field_name::FieldName, crate::schema::record::field::Field>,
    pub open: bool,
    pub additional: Option<Box<crate::schema::core::type_node::Type>>,
    pub required_order: Option<Vec<crate::schema::record::field_name::FieldName>>,
}
