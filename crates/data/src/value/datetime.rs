use facet::Facet;

#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime(time::OffsetDateTime);

impl DateTime {
    pub fn now_utc() -> Self {
        DateTime(time::OffsetDateTime::now_utc())
    }
}

impl From<time::OffsetDateTime> for DateTime {
    fn from(datetime: time::OffsetDateTime) -> Self {
        Self(datetime)
    }
}

impl From<DateTime> for time::OffsetDateTime {
    fn from(datetime: DateTime) -> Self {
        datetime.0
    }
}
