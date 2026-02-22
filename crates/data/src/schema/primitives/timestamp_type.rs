#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct TimestampType {
    pub unit: crate::schema::primitives::time_unit::TimeUnit,
    pub timezone: crate::schema::primitives::time_zone_spec::TimeZoneSpec,
}
