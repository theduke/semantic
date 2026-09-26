/// Wire-compatible representation used only to read schemas written before
/// entity references became intrinsic foreign keys.
#[doc(hidden)]
#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct LegacyReferenceConstraint {
    pub to: crate::schema::core::type_ref::TypeRef,
    pub fields: Vec<crate::schema::record::field_name::FieldName>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Constraint {
    Min(crate::schema::constraints::number_bound::NumberBound),
    Max(crate::schema::constraints::number_bound::NumberBound),
    MultipleOf(String),

    Length(crate::schema::constraints::length_spec::LengthSpec),
    Pattern(String),
    Prefix(String),
    Suffix(String),
    Contains(crate::value::Value),

    Precision {
        precision: u32,
        scale: u32,
    },
    Charset(crate::schema::constraints::charset::Charset),
    Collation(String),
    TimeZone(crate::schema::primitives::time_zone_spec::TimeZoneSpec),

    MinItems(u64),
    MaxItems(u64),
    MinProperties(u64),
    MaxProperties(u64),

    RequiredFields(Vec<crate::schema::record::field_name::FieldName>),
    KeyPattern(String),

    Unique,
    Distinct,

    PrimaryKey,
    #[doc(hidden)]
    #[facet(rename = "foreign_key")]
    LegacyForeignKey(LegacyReferenceConstraint),
    Index {
        name: Option<String>,
        fields: Vec<crate::schema::record::field_name::FieldName>,
        unique: bool,
    },

    DefaultValue {
        value: crate::value::Value,
    },
    DefaultExpr {
        expr: crate::expr::Expr,
    },

    Transport {
        format: crate::schema::constraints::transport_format::TransportFormat,
        media_type: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_legacy_foreign_key_for_catalog_upgrade() {
        let constraint: Constraint = facet_json::from_str(
            r#"{"foreign_key":{"to":{"name":"example:Person","args":[]},"fields":["id"]}}"#,
        )
        .unwrap();
        let Constraint::LegacyForeignKey(reference) = constraint else {
            panic!("expected legacy foreign key");
        };
        assert_eq!(reference.to.name, "example:Person");
        assert_eq!(reference.fields, ["id"]);
    }
}
