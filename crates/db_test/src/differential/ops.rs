//! Random operations of the differential test.

use semantic_data::query::{Assignment, BatchOperation, BinaryOp, DeleteQuery, Expr, UpdateQuery};
use semantic_data::value::{FieldPath, Object, Value};

use super::model::Model;
use super::schema::{
    AUTHOR, CLASSES, DOC, IndexSpec, LABEL, NOTE, PERSON, PLAIN, SUBJECT, Schema, binary, field,
    literal,
};
use crate::rng::TestRng;

const NAMES: u64 = 8;
const IDS: u64 = 24;
const TAGS: [&str; 3] = ["red", "green", "blue"];
const WORDS: [&str; 8] = [
    "red", "fox", "dog", "jumps", "over", "lazy", "Quick", "brown",
];

/// One step of an interactive transaction.
#[derive(Debug, Clone)]
pub(super) enum TxStep {
    Upsert {
        collection: String,
        id: String,
        object: Object,
    },
    Delete {
        collection: String,
        id: String,
    },
    Update(UpdateQuery),
    Select(String),
    Savepoint,
    /// Roll back to the savepoint at this position of the savepoint stack
    /// (modulo its size); no-op without savepoints.
    RollbackTo(usize),
}

#[derive(Debug, Clone)]
pub(super) enum Op {
    /// By-id writes to the plain collections, predicted by the model.
    Batch(Vec<BatchOperation>),
    /// Writes of class rows; reference validation decides their outcome.
    ClassBatch(Vec<BatchOperation>),
    Update(UpdateQuery),
    Delete(DeleteQuery),
    Transaction {
        steps: Vec<TxStep>,
        commit: bool,
    },
    AddIndex(IndexSpec),
    DropIndex(IndexSpec),
    Reopen,
    /// A write every engine must reject.
    Invalid(Vec<BatchOperation>),
}

pub(super) fn plain_id(rng: &mut TestRng, collection: &str) -> String {
    format!("{}{}", &collection[5..], rng.below(IDS))
}

fn words(rng: &mut TestRng) -> String {
    (0..1 + rng.below(4))
        .map(|_| *rng.pick(&WORDS))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn plain_row(rng: &mut TestRng, collection: &str, id: &str) -> Object {
    let mut row = Object::new();
    row.insert("id", Value::String(id.to_string()));
    if !rng.one_in(6) {
        row.insert("nick", Value::String(format!("n{}", rng.below(NAMES))));
    }
    match rng.below(20) {
        0 | 1 => {}
        2 => {
            row.insert("n", Value::Null);
        }
        _ => {
            row.insert("n", Value::I64(rng.below(20) as i64 - 5));
        }
    }
    if !rng.one_in(10) {
        row.insert("grp", Value::I64(rng.below(3) as i64));
    }
    if !rng.one_in(5) {
        row.insert("tag", Value::String(rng.pick(&TAGS).to_string()));
    }
    if !rng.one_in(4) {
        row.insert("body", Value::String(words(rng)));
    }
    if rng.one_in(2) {
        let other = PLAIN[usize::from(collection == PLAIN[0])];
        row.insert("ref", Value::String(plain_id(rng, other)));
    }
    if rng.one_in(4) {
        // Fresh field names grow the field dictionaries.
        let mut extra = Object::new();
        for _ in 0..1 + rng.below(3) {
            extra.insert(
                format!("x{}", rng.below(40)),
                Value::I64(rng.below(9) as i64),
            );
        }
        row.insert("extra", Value::Object(extra));
    }
    row
}

/// Ids of existing rows are chosen more often than fresh ones.
fn target_id(rng: &mut TestRng, model: &Model, collection: &str) -> String {
    let ids = model.ids(collection);
    if !ids.is_empty() && rng.below(3) != 0 {
        rng.pick(&ids).clone()
    } else {
        plain_id(rng, collection)
    }
}

fn by_id_op(rng: &mut TestRng, model: &Model) -> BatchOperation {
    let collection = rng.pick(&PLAIN).to_string();
    let id = target_id(rng, model, &collection);
    match rng.below(10) {
        0..=4 => BatchOperation::Upsert {
            object: plain_row(rng, &collection, &id),
            collection,
            id,
        },
        5 | 6 => BatchOperation::Create {
            object: plain_row(rng, &collection, &id),
            collection,
            id,
        },
        7 | 8 => BatchOperation::DeleteById { collection, id },
        _ => BatchOperation::DeleteByIds {
            ids: (0..1 + rng.below(3))
                .map(|_| target_id(rng, model, &collection))
                .collect(),
            collection,
        },
    }
}

/// A predicate over the plain row fields.
pub(super) fn predicate(rng: &mut TestRng) -> Expr {
    let n = || field("n");
    match rng.below(8) {
        0 => binary(
            BinaryOp::Eq,
            field("nick"),
            literal(format!("n{}", rng.below(NAMES))),
        ),
        1 => binary(BinaryOp::Gt, n(), literal(rng.below(15) as i64 - 3)),
        2 => binary(
            BinaryOp::And,
            binary(BinaryOp::Gte, n(), literal(rng.below(6) as i64)),
            binary(BinaryOp::Lte, n(), literal(rng.below(10) as i64 + 3)),
        ),
        3 => binary(
            BinaryOp::And,
            binary(BinaryOp::Eq, field("grp"), literal(rng.below(3) as i64)),
            binary(BinaryOp::Gte, n(), literal(rng.below(8) as i64)),
        ),
        4 => binary(
            BinaryOp::And,
            binary(
                BinaryOp::Eq,
                field("tag"),
                literal(rng.pick(&TAGS).to_string()),
            ),
            binary(BinaryOp::Gt, n(), literal(0i64)),
        ),
        5 => Expr::TextMatch {
            exprs: vec![field("body")],
            query: Box::new(literal(words(rng))),
            mode: if rng.one_in(2) {
                semantic_data::query::TextMatchMode::All
            } else {
                semantic_data::query::TextMatchMode::Any
            },
            analyzer: Default::default(),
        },
        6 => Expr::InList {
            expr: Box::new(field("nick")),
            list: (0..1 + rng.below(3))
                .map(|_| literal(format!("n{}", rng.below(NAMES))))
                .collect(),
            negated: rng.one_in(3),
        },
        _ => binary(
            BinaryOp::Or,
            binary(BinaryOp::Eq, field("tag"), literal("green".to_string())),
            binary(BinaryOp::Lt, n(), literal(rng.below(5) as i64 - 3)),
        ),
    }
}

fn assignment(rng: &mut TestRng) -> Assignment {
    let (path, value) = match rng.below(5) {
        0 => ("tag", literal(rng.pick(&TAGS).to_string())),
        1 => ("n", binary(BinaryOp::Add, field("n"), literal(1i64))),
        2 => ("nick", literal(format!("n{}", rng.below(NAMES)))),
        3 => ("grp", literal(rng.below(3) as i64)),
        _ => ("body", literal(words(rng))),
    };
    Assignment {
        path: FieldPath::from_fields([path]),
        value,
    }
}

fn update(rng: &mut TestRng) -> UpdateQuery {
    let mut query = UpdateQuery::new()
        .with_collection(*rng.pick(&PLAIN))
        .with_predicate(predicate(rng));
    query.assignments = (0..1 + rng.below(2)).map(|_| assignment(rng)).collect();
    query
}

fn class_op(rng: &mut TestRng, model: &Model) -> BatchOperation {
    let existing = |rng: &mut TestRng, prefix: &str| {
        let ids = model
            .ids(CLASSES)
            .into_iter()
            .filter(|id| id.starts_with(prefix))
            .collect::<Vec<_>>();
        if !ids.is_empty() && !rng.one_in(4) {
            rng.pick(&ids).clone()
        } else {
            format!("{prefix}{}", rng.below(8))
        }
    };
    let (class, prefix) = *rng.pick(&[(PERSON, "p"), (DOC, "d"), (NOTE, "t")]);
    let id = existing(rng, prefix);
    if rng.one_in(4) {
        return BatchOperation::DeleteById {
            collection: CLASSES.into(),
            id,
        };
    }
    let mut row = Object::new();
    row.insert("id", Value::String(id.clone()));
    row.insert("type", Value::String(class.to_string()));
    if class != NOTE && !rng.one_in(4) {
        row.insert(LABEL, Value::String(format!("L{}", rng.below(6))));
    }
    // References usually point to existing targets; dangling ones must be
    // rejected by reference validation.
    match class {
        DOC if !rng.one_in(5) => {
            row.insert(AUTHOR, Value::String(existing(rng, "p")));
        }
        NOTE if !rng.one_in(5) => {
            row.insert(SUBJECT, Value::String(existing(rng, "d")));
        }
        _ => {}
    }
    BatchOperation::Upsert {
        collection: CLASSES.into(),
        id,
        object: row,
    }
}

fn transaction(rng: &mut TestRng, model: &Model) -> Op {
    let steps = (0..2 + rng.below(6))
        .map(|_| {
            let collection = rng.pick(&PLAIN).to_string();
            match rng.below(10) {
                0..=3 => {
                    let id = target_id(rng, model, &collection);
                    TxStep::Upsert {
                        object: plain_row(rng, &collection, &id),
                        collection,
                        id,
                    }
                }
                4 => TxStep::Delete {
                    id: target_id(rng, model, &collection),
                    collection,
                },
                5 => TxStep::Update(update(rng)),
                6 => TxStep::Select(collection),
                7 | 8 => TxStep::Savepoint,
                _ => TxStep::RollbackTo(rng.index(4)),
            }
        })
        .collect();
    Op::Transaction {
        steps,
        commit: !rng.one_in(4),
    }
}

fn invalid(rng: &mut TestRng, model: &Model) -> Op {
    let collection = rng.pick(&PLAIN).to_string();
    let id = plain_id(rng, &collection);
    let op = match rng.below(3) {
        0 => BatchOperation::Upsert {
            collection: "diff_missing".into(),
            object: plain_row(rng, &collection, &id),
            id,
        },
        1 => {
            // The id field must match the row id.
            let mut object = plain_row(rng, &collection, &id);
            object.insert("id", Value::String(format!("{id}-other")));
            BatchOperation::Upsert {
                collection,
                id,
                object,
            }
        }
        _ => {
            let ids = model.ids(&collection);
            let Some(id) = ids.first().cloned() else {
                return Op::Invalid(vec![BatchOperation::DeleteById {
                    collection: "diff_missing".into(),
                    id,
                }]);
            };
            BatchOperation::Create {
                object: plain_row(rng, &collection, &id),
                collection,
                id,
            }
        }
    };
    Op::Invalid(vec![op])
}

pub(super) fn random_op(rng: &mut TestRng, model: &Model, schema: &Schema) -> Op {
    match rng.below(100) {
        0..=29 => Op::Batch(vec![by_id_op(rng, model)]),
        30..=41 => Op::Batch(
            (0..2 + rng.below(3))
                .map(|_| by_id_op(rng, model))
                .collect(),
        ),
        42..=51 => Op::Update(update(rng)),
        52..=56 => Op::Delete(
            DeleteQuery::new()
                .with_collection(*rng.pick(&PLAIN))
                .with_predicate(predicate(rng)),
        ),
        57..=68 => Op::ClassBatch(
            (0..1 + rng.below(3))
                .map(|_| class_op(rng, model))
                .collect(),
        ),
        69..=80 => transaction(rng, model),
        81..=84 => {
            let absent = schema
                .menu
                .iter()
                .filter(|index| !schema.active.contains(index))
                .cloned()
                .collect::<Vec<_>>();
            if absent.is_empty() || (!schema.active.is_empty() && rng.one_in(2)) {
                Op::DropIndex(rng.pick(&schema.active).clone())
            } else {
                Op::AddIndex(rng.pick(&absent).clone())
            }
        }
        85..=87 => Op::Reopen,
        _ => invalid(rng, model),
    }
}
