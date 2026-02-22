#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum TemporalType {
    Date,
    Time,
    DateTime,
    Duration,
    Period,
    Timestamp(crate::schema::primitives::timestamp_type::TimestampType),
    Instant,
}
