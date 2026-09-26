//! Row ordering for the `Sort` and `TopN` operators.
//!
//! Every row's `ORDER BY` values are evaluated once into a [`SortKey`]
//! instead of on every comparison. Rows with equal keys keep their input
//! order, so both operators behave like a stable sort.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use futures::TryStreamExt;
use semantic_data::query::SortDirection;
use semantic_data::value::Value;

use super::{DEFAULT_EXECUTION_BATCH_SIZE, DynObject, RecordBatchStream};
use crate::plan::PhysicalOrderField;
use crate::query::{CoreResult, ObjectAccess, evaluate_expr};

/// Evaluated `ORDER BY` values of one row.
///
/// Orders exactly like `compare_objects_for_plan`: columns compare in order,
/// a missing value sorts before any present one, and descending columns
/// reverse the column ordering.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SortKey(Vec<SortKeyPart>);

impl SortKey {
    fn of(row: &dyn ObjectAccess, order_by: &[PhysicalOrderField]) -> Self {
        Self(
            order_by
                .iter()
                .map(|field| SortKeyPart {
                    value: evaluate_expr(row, &field.expr),
                    descending: matches!(field.direction, SortDirection::Desc),
                })
                .collect(),
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
struct SortKeyPart {
    value: Option<Value>,
    descending: bool,
}

impl Ord for SortKeyPart {
    fn cmp(&self, other: &Self) -> Ordering {
        let ordering = self.value.cmp(&other.value);
        if self.descending {
            ordering.reverse()
        } else {
            ordering
        }
    }
}

impl PartialOrd for SortKeyPart {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A row with its sort key and input position, ordered by key and then by
/// position so that ties keep input order.
struct KeyedRow {
    key: SortKey,
    position: u64,
    row: DynObject,
}

impl KeyedRow {
    fn new(row: DynObject, position: u64, order_by: &[PhysicalOrderField]) -> Self {
        Self {
            key: SortKey::of(row.as_ref(), order_by),
            position,
            row,
        }
    }
}

impl Ord for KeyedRow {
    fn cmp(&self, other: &Self) -> Ordering {
        self.key
            .cmp(&other.key)
            .then(self.position.cmp(&other.position))
    }
}

impl PartialOrd for KeyedRow {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for KeyedRow {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for KeyedRow {}

/// Collect `input` and return its rows stably sorted by `order_by`.
pub(super) async fn sort_rows(
    mut input: RecordBatchStream<'_>,
    order_by: &[PhysicalOrderField],
) -> CoreResult<Vec<DynObject>> {
    let mut rows = Vec::new();
    while let Some(batch) = input.try_next().await? {
        for row in batch {
            let position = rows.len() as u64;
            rows.push(KeyedRow::new(row, position, order_by));
        }
    }
    // Keys are unique through the position, so an unstable sort is stable.
    rows.sort_unstable();
    Ok(rows.into_iter().map(|row| row.row).collect())
}

/// Bounded selection of the first `capacity` rows of an ordering.
///
/// Keeps at most `capacity` rows in a max-heap whose top is the last
/// retained row; a new row replaces it only when it orders strictly before
/// it, which keeps the earliest input rows among ties.
pub(super) struct TopN {
    order_by: Vec<PhysicalOrderField>,
    capacity: usize,
    heap: BinaryHeap<KeyedRow>,
    next_position: u64,
}

impl TopN {
    pub(super) fn new(order_by: Vec<PhysicalOrderField>, capacity: usize) -> Self {
        Self {
            order_by,
            capacity,
            heap: BinaryHeap::with_capacity(capacity.min(DEFAULT_EXECUTION_BATCH_SIZE)),
            next_position: 0,
        }
    }

    pub(super) fn push(&mut self, row: DynObject) {
        if self.capacity == 0 {
            return;
        }
        let entry = KeyedRow::new(row, self.next_position, &self.order_by);
        self.next_position += 1;
        if self.heap.len() < self.capacity {
            self.heap.push(entry);
            record_retained_rows(self.heap.len());
        } else if let Some(mut last) = self.heap.peek_mut()
            && entry < *last
        {
            *last = entry;
        }
    }

    /// Consume `input` into the selection.
    pub(super) async fn extend(&mut self, mut input: RecordBatchStream<'_>) -> CoreResult<()> {
        while let Some(batch) = input.try_next().await? {
            for row in batch {
                self.push(row);
            }
        }
        Ok(())
    }

    /// Number of rows currently retained (at most the capacity).
    pub(super) fn retained(&self) -> usize {
        self.heap.len()
    }

    /// The retained rows in order, without the first `offset`.
    pub(super) fn finish(self, offset: usize) -> Vec<DynObject> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .skip(offset)
            .map(|entry| entry.row)
            .collect()
    }
}

#[cfg(test)]
thread_local! {
    /// Largest number of rows a `TopN` retained on this thread since the
    /// last [`take_top_n_peak_rows`].
    static TOP_N_PEAK_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_retained_rows(rows: usize) {
    TOP_N_PEAK_ROWS.with(|peak| peak.set(peak.get().max(rows)));
}

#[cfg(not(test))]
fn record_retained_rows(_rows: usize) {}

/// Return and reset the largest number of rows a `TopN` operator retained on
/// the current thread.
#[cfg(test)]
pub(crate) fn take_top_n_peak_rows() -> usize {
    TOP_N_PEAK_ROWS.with(|peak| peak.replace(0))
}

#[cfg(test)]
mod tests {
    use futures::{StreamExt, executor::block_on, stream};
    use semantic_data::query::SortDirection;
    use semantic_data::value::{FieldPath, Object, Value};

    use super::{TopN, sort_rows, take_top_n_peak_rows};
    use crate::plan::PhysicalOrderField;
    use crate::plan::execute::{DynObject, rows_to_batches};
    use crate::query::{Expr, Operand, OrderBy, compare_objects_for_plan};

    /// Deterministic splitmix64 generator for reproducible random data.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        fn below(&mut self, bound: u64) -> u64 {
            self.next() % bound
        }
    }

    /// A value with frequent ties, nulls, missing fields and mixed types.
    fn random_value(rng: &mut Rng) -> Option<Value> {
        match rng.below(8) {
            0 => None,
            1 => Some(Value::Null),
            2 => Some(Value::String(format!("s{}", rng.below(3)))),
            3 => Some(Value::F64((rng.below(4) as f64 / 2.0).into())),
            4 => Some(Value::Bool(rng.below(2) == 0)),
            _ => Some(Value::I64(rng.below(5) as i64)),
        }
    }

    fn random_rows(rng: &mut Rng, count: usize) -> Vec<Object> {
        (0..count)
            .map(|position| {
                let mut row = Object::new();
                row.insert("position", Value::I64(position as i64));
                for field in ["a", "b"] {
                    if let Some(value) = random_value(rng) {
                        row.insert(field, value);
                    }
                }
                row
            })
            .collect()
    }

    fn order(field: &str, direction: SortDirection) -> PhysicalOrderField {
        PhysicalOrderField {
            expr: Expr::Operand(Operand::Field(FieldPath::from_fields([field]))),
            direction,
        }
    }

    fn orderings() -> Vec<Vec<PhysicalOrderField>> {
        use SortDirection::{Asc, Desc};
        vec![
            vec![order("a", Asc)],
            vec![order("a", Desc)],
            vec![order("a", Asc), order("b", Desc)],
            vec![order("b", Desc), order("a", Asc)],
            vec![order("missing", Asc)],
        ]
    }

    /// Stable sort with the row comparator of the former `Sort` operator.
    fn reference_sorted(rows: &[Object], order_by: &[PhysicalOrderField]) -> Vec<Object> {
        let legacy = order_by
            .iter()
            .map(|field| OrderBy {
                expr: field.expr.clone(),
                direction: field.direction,
            })
            .collect::<Vec<_>>();
        let mut sorted = rows.to_vec();
        sorted.sort_by(|a, b| compare_objects_for_plan(a, b, &legacy));
        sorted
    }

    fn dyn_rows(rows: &[Object]) -> Vec<DynObject> {
        rows.iter()
            .cloned()
            .map(|row| Box::new(row) as DynObject)
            .collect()
    }

    fn objects(rows: Vec<DynObject>) -> Vec<Object> {
        rows.into_iter().map(|row| row.to_object()).collect()
    }

    #[test]
    fn sort_matches_stable_comparator_sort() {
        let mut rng = Rng(7);
        for count in [0, 1, 2, 17, 300] {
            let rows = random_rows(&mut rng, count);
            for order_by in orderings() {
                let sorted =
                    block_on(sort_rows(rows_to_batches(dyn_rows(&rows), 7), &order_by)).unwrap();
                assert_eq!(objects(sorted), reference_sorted(&rows, &order_by));
            }
        }
    }

    #[test]
    fn top_n_matches_sort_then_limit() {
        let mut rng = Rng(42);
        for round in 0..40 {
            let count = rng.below(200) as usize;
            let rows = random_rows(&mut rng, count);
            for order_by in orderings() {
                let offset = if round % 2 == 0 {
                    0
                } else {
                    rng.below(20) as usize
                };
                let limit = rng.below(25) as usize;
                let expected = reference_sorted(&rows, &order_by)
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .collect::<Vec<_>>();

                let mut top_n = TopN::new(order_by.clone(), offset + limit);
                for row in dyn_rows(&rows) {
                    top_n.push(row);
                }
                assert_eq!(
                    objects(top_n.finish(offset)),
                    expected,
                    "round {round}, offset {offset}, limit {limit}, order {order_by:?}"
                );
            }
        }
    }

    #[test]
    fn top_n_retains_at_most_offset_plus_limit_rows() {
        let mut rng = Rng(3);
        let rows = random_rows(&mut rng, 5_000);
        take_top_n_peak_rows();
        let mut top_n = TopN::new(orderings().remove(0), 5 + 10);
        block_on(top_n.extend(rows_to_batches(dyn_rows(&rows), 1024))).unwrap();
        assert_eq!(top_n.finish(5).len(), 10);
        assert_eq!(take_top_n_peak_rows(), 15);
    }

    #[test]
    fn top_n_with_zero_capacity_retains_nothing() {
        let mut top_n = TopN::new(orderings().remove(0), 0);
        block_on(top_n.extend(stream::iter([Ok(dyn_rows(&random_rows(&mut Rng(1), 3)))]).boxed()))
            .unwrap();
        assert!(top_n.finish(0).is_empty());
    }
}
