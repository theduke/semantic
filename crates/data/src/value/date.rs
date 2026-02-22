#[derive(facet::Facet, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[facet(transparent)]
pub struct Date(#[facet(opaque, proxy = FacetProxy)] time::Date);

impl Date {
    pub fn now_utc() -> Self {
        Self(time::OffsetDateTime::now_utc().date())
    }
}

impl From<time::Date> for Date {
    fn from(date: time::Date) -> Self {
        Self(date)
    }
}

impl From<Date> for time::Date {
    fn from(date: Date) -> Self {
        date.0
    }
}

#[derive(facet::Facet)]
#[facet(transparent)]
struct FacetProxy(time::OffsetDateTime);

impl TryFrom<FacetProxy> for time::Date {
    type Error = &'static str;

    fn try_from(proxy: FacetProxy) -> Result<Self, Self::Error> {
        Ok(proxy.0.date())
    }
}

impl From<&time::Date> for FacetProxy {
    fn from(key: &time::Date) -> Self {
        let dt = time::OffsetDateTime::new_utc(*key, time::Time::MIDNIGHT);
        Self(dt)
    }
}
