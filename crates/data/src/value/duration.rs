#[derive(facet::Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[facet(transparent)]
pub struct Duration(
    /// Uses time::Duration internally, but does not expose it directly
    /// for public API stability.
    #[facet(opaque, proxy = FacetProxy)]
    time::Duration,
);

impl From<time::Duration> for Duration {
    fn from(duration: time::Duration) -> Self {
        Self(duration)
    }
}

impl From<Duration> for time::Duration {
    fn from(span: Duration) -> Self {
        span.0
    }
}

#[derive(facet::Facet, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[facet(untagged)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
enum FacetProxy {
    Seconds(i64),
}

impl TryFrom<FacetProxy> for time::Duration {
    type Error = &'static str;

    fn try_from(proxy: FacetProxy) -> Result<Self, Self::Error> {
        match proxy {
            FacetProxy::Seconds(seconds) => Ok(time::Duration::new(seconds, 0)),
        }
    }
}

impl From<&time::Duration> for FacetProxy {
    fn from(key: &time::Duration) -> Self {
        Self::Seconds(key.whole_seconds())
    }
}
