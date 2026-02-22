#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum UIntWidth {
    U8,
    U16,
    U24,
    U32,
    U40,
    U48,
    U56,
    U64,
    U128,
    U256,
}
