//! Key ranges of equality and range index scans.
//!
//! Index keys of equality and range indexes are derived by
//! [`IndexSchema::key_value`](crate::catalog::IndexSchema::key_value): the
//! column value of a single-column index, or the list of column values of a
//! composite index. Storages keep the entries of one index ordered by
//! (key value, entity id) under `Value` order, so the ranges below describe
//! contiguous runs of entries.

use std::cmp::Ordering;
use std::ops::Bound;

use semantic_data::value::Value;

/// Range of the key column following the fixed prefix of an
/// [`IndexScanRange`].
#[derive(Debug, Clone, PartialEq)]
pub enum IndexColumnRange {
    /// Values within `lower..upper` by `Value` order.
    Bounds {
        lower: Bound<Value>,
        upper: Bound<Value>,
    },
    /// Strings starting with the prefix.
    StringPrefix(String),
}

impl IndexColumnRange {
    /// Every value.
    pub fn all() -> Self {
        Self::Bounds {
            lower: Bound::Unbounded,
            upper: Bound::Unbounded,
        }
    }

    /// Exactly `value`.
    pub fn eq(value: Value) -> Self {
        Self::Bounds {
            lower: Bound::Included(value.clone()),
            upper: Bound::Included(value),
        }
    }

    pub fn contains(&self, value: &Value) -> bool {
        match self {
            Self::Bounds { lower, upper } => {
                let above = match lower {
                    Bound::Unbounded => true,
                    Bound::Included(lower) => value >= lower,
                    Bound::Excluded(lower) => value > lower,
                };
                let below = match upper {
                    Bound::Unbounded => true,
                    Bound::Included(upper) => value <= upper,
                    Bound::Excluded(upper) => value < upper,
                };
                above && below
            }
            Self::StringPrefix(prefix) => {
                matches!(value, Value::String(value) if value.starts_with(prefix.as_str()))
            }
        }
    }

    /// Whether no value lies in the range.
    pub fn is_empty(&self) -> bool {
        let Self::Bounds { lower, upper } = self else {
            return false;
        };
        let (lower, lower_inclusive) = match lower {
            Bound::Unbounded => return false,
            Bound::Included(value) => (value, true),
            Bound::Excluded(value) => (value, false),
        };
        let (upper, upper_inclusive) = match upper {
            Bound::Unbounded => return false,
            Bound::Included(value) => (value, true),
            Bound::Excluded(value) => (value, false),
        };
        match lower.cmp(upper) {
            Ordering::Greater => true,
            Ordering::Equal => !(lower_inclusive && upper_inclusive),
            Ordering::Less => false,
        }
    }
}

/// Contiguous run of index entries: equality on the leading key columns and
/// a range on the next one.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexScanRange {
    /// Whether the index is composite (keys are lists of column values).
    pub composite: bool,
    /// Values of the leading key columns, fixed by equality. Always empty
    /// for single-column indexes.
    pub prefix: Vec<Value>,
    /// Range of the key column after `prefix`. Ignored when `prefix` fixes
    /// every column (the scan then covers exactly the prefix).
    pub column: IndexColumnRange,
}

impl IndexScanRange {
    /// Entries of a single-column index whose value lies in `column`.
    pub fn single(column: IndexColumnRange) -> Self {
        Self {
            composite: false,
            prefix: Vec::new(),
            column,
        }
    }

    /// Whether the index key `key` (see
    /// [`IndexSchema::key_value`](crate::catalog::IndexSchema::key_value))
    /// lies in this range.
    pub fn contains(&self, key: &Value) -> bool {
        let columns = if self.composite {
            match key {
                Value::List(columns) => columns.as_slice(),
                _ => return false,
            }
        } else {
            std::slice::from_ref(key)
        };
        if columns.len() < self.prefix.len()
            || columns.iter().zip(&self.prefix).any(|(a, b)| a != b)
        {
            return false;
        }
        columns
            .get(self.prefix.len())
            .is_none_or(|value| self.column.contains(value))
    }
}

/// One scan of an index: a key range, its direction, and whether entries
/// carry their decoded key value.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexScan {
    pub range: IndexScanRange,
    /// Iterate in descending (key value, id) order.
    pub reverse: bool,
    /// Decode the key value of each entry (for index-only reads).
    pub with_keys: bool,
}

/// An index entry returned by an [`IndexScan`].
#[derive(Debug, Clone, PartialEq)]
pub struct IndexEntry {
    pub id: String,
    /// The decoded key value, when requested with
    /// [`IndexScan::with_keys`]. Decoding canonicalizes what the key
    /// encoding does not preserve: `-0.0` decodes as `0.0`, NaN payloads are
    /// lost and `DateTime` values decode with a UTC offset.
    pub key: Option<Value>,
}

/// Whether decoding a key holding `value` reproduces it exactly (see
/// [`IndexEntry::key`]).
pub fn key_decodes_losslessly(value: &Value) -> bool {
    match value {
        Value::F32(_) | Value::F64(_) | Value::DateTime(_) => false,
        Value::List(items) => items.iter().all(key_decodes_losslessly),
        Value::Map(entries) => entries
            .iter()
            .all(|(key, value)| key_decodes_losslessly(key) && key_decodes_losslessly(value)),
        Value::Object(object) => object.values().all(key_decodes_losslessly),
        Value::Variant(variant) => key_decodes_losslessly(&variant.value),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(value: &str) -> Value {
        Value::String(value.to_string())
    }

    #[test]
    fn column_ranges_follow_value_order() {
        let range = IndexColumnRange::Bounds {
            lower: Bound::Excluded(Value::I64(2)),
            upper: Bound::Included(Value::I64(5)),
        };
        assert!(!range.contains(&Value::I64(2)));
        assert!(range.contains(&Value::I64(3)));
        assert!(range.contains(&Value::I64(5)));
        assert!(!range.contains(&Value::I64(6)));
        // Cross-type order is the variant rank, like comparisons in filters.
        assert!(!range.contains(&Value::U64(3)));
        assert!(!range.is_empty());
        assert!(IndexColumnRange::eq(Value::I64(1)).contains(&Value::I64(1)));
        assert!(
            IndexColumnRange::Bounds {
                lower: Bound::Included(Value::I64(3)),
                upper: Bound::Excluded(Value::I64(3)),
            }
            .is_empty()
        );
        let prefix = IndexColumnRange::StringPrefix("ab".to_string());
        assert!(prefix.contains(&s("abc")));
        assert!(!prefix.contains(&s("a")));
        assert!(!prefix.contains(&Value::I64(1)));
    }

    #[test]
    fn composite_ranges_fix_leading_columns() {
        let range = IndexScanRange {
            composite: true,
            prefix: vec![s("a")],
            column: IndexColumnRange::Bounds {
                lower: Bound::Excluded(Value::Void),
                upper: Bound::Excluded(Value::I64(10)),
            },
        };
        assert!(range.contains(&Value::List(vec![s("a"), Value::I64(3), s("x")])));
        assert!(!range.contains(&Value::List(vec![s("b"), Value::I64(3), s("x")])));
        assert!(!range.contains(&Value::List(vec![s("a"), Value::Void, s("x")])));
        assert!(!range.contains(&s("a")));
    }
}
