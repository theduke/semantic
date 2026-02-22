#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum FloatWidth {
    F16,
    F32,
    F64,
    F80,
    F128,
    Decimal32,
    Decimal64,
    Decimal128,
}
