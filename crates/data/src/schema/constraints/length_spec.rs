#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum LengthSpec {
    Exactly(u64),
    Range { min: Option<u64>, max: Option<u64> },
}

impl LengthSpec {
    /// Normalizes the `LengthSpec` to the canonical form.
    ///
    /// Self::Exactly(n) is the canonical form if min and max are equal.
    pub fn normalize(&mut self) {
        match self {
            LengthSpec::Range {
                min: Some(min),
                max: Some(max),
            } if min == max => {
                *self = LengthSpec::Exactly(*min);
            }
            _ => {}
        }
    }
}
