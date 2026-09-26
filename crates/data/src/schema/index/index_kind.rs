#[derive(facet::Facet, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum IndexKind {
    /// Equality lookups (`=`, `IN`) on the key columns.
    #[default]
    Equality,
    PathEquality,
    /// Ordered index: equality plus range, prefix and ordered scans.
    Range,
    FullText,
}

impl IndexKind {
    pub fn is_equality(&self) -> bool {
        *self == Self::Equality
    }

    /// Whether index keys are derived from the key column values (equality
    /// and range indexes share one key derivation).
    pub fn is_value_index(&self) -> bool {
        matches!(self, Self::Equality | Self::Range)
    }
}
