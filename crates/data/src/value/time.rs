use crate::Duration;

#[derive(facet::Facet, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[facet(transparent)]
pub struct Time(#[facet(opaque, proxy = FacetProxy)] time::Time);

impl Time {
    pub fn now_utc() -> Self {
        Self(time::OffsetDateTime::now_utc().time())
    }

    pub fn now_local() -> Self {
        todo!()
    }
}

impl From<time::Time> for Time {
    fn from(time: time::Time) -> Self {
        Self(time)
    }
}

impl From<Time> for time::Time {
    fn from(time: Time) -> Self {
        time.0
    }
}

#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[facet(transparent)]
struct FacetProxy(Duration);

impl TryFrom<FacetProxy> for time::Time {
    type Error = &'static str;

    fn try_from(proxy: FacetProxy) -> Result<Self, Self::Error> {
        let dur: time::Duration = proxy.0.into();
        Ok(time::Time::MIDNIGHT + dur)
    }
}

impl From<&time::Time> for FacetProxy {
    fn from(key: &time::Time) -> Self {
        let dur = *key - time::Time::MIDNIGHT;
        Self(dur.into())
    }
}
