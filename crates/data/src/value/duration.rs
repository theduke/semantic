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
    Milliseconds(i64),
}

impl TryFrom<FacetProxy> for time::Duration {
    type Error = &'static str;

    fn try_from(proxy: FacetProxy) -> Result<Self, Self::Error> {
        match proxy {
            FacetProxy::Milliseconds(milliseconds) => {
                Ok(time::Duration::milliseconds(milliseconds))
            }
        }
    }
}

impl From<&time::Duration> for FacetProxy {
    fn from(key: &time::Duration) -> Self {
        Self::Milliseconds(key.whole_milliseconds() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::Duration;

    #[test]
    fn facet_json_serializes_milliseconds() {
        let duration = Duration::from(time::Duration::milliseconds(1500));
        let encoded = facet_json::to_string(&duration).expect("serialize duration");
        assert_eq!(encoded, "1500");
    }

    #[test]
    fn facet_proxy_truncates_sub_millisecond_precision() {
        let duration = Duration::from(time::Duration::microseconds(1500));
        let encoded = facet_json::to_string(&duration).expect("serialize duration");
        assert_eq!(encoded, "1");
    }
}
