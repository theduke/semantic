#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum TimeUnit {
    Seconds,
    Millis,
    Micros,
    Nanos,
}
