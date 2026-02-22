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
    Contains(crate::schema::core::literal_value::LiteralValue),

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
    ForeignKey(crate::schema::constraints::foreign_key_ref::ForeignKeyRef),
    Index {
        name: Option<String>,
        fields: Vec<crate::schema::record::field_name::FieldName>,
        unique: bool,
    },

    DefaultValue {
        value: crate::schema::core::literal_value::LiteralValue,
    },
    DefaultExpr {
        expr: crate::expr::Expr,
    },

    Transport {
        format: crate::schema::constraints::transport_format::TransportFormat,
        media_type: Option<String>,
    },
}
