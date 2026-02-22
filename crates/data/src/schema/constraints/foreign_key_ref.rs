#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ForeignKeyRef {
    pub to: crate::schema::core::type_ref::TypeRef,
    pub fields: Vec<crate::schema::record::field_name::FieldName>,
}
