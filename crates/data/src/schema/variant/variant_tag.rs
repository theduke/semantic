#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum VariantTag {
    ExternallyTagged,
    InternallyTagged {
        field: crate::schema::record::field_name::FieldName,
    },
    AdjacentlyTagged {
        tag_field: crate::schema::record::field_name::FieldName,
        data_field: crate::schema::record::field_name::FieldName,
    },
    Untagged,
}
