#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum IntWidth {
    I8,
    I16,
    I24,
    I32,
    I40,
    I48,
    I56,
    I64,
    I128,
    I256,
}
