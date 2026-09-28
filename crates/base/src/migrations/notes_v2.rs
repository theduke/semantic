//! Frozen Note class definition recorded by migration 011.

use semantic_data::{
    expr::{Expr, LiteralExpr},
    schema::ClassType,
    value::Value,
};

pub(super) fn class() -> ClassType {
    let mut class = super::notes_v1::class();
    class
        .attributes
        .get_mut("note_format")
        .expect("original Note class has a format field")
        .default = Some(Expr::Literal(LiteralExpr {
        value: Value::String("markdown".to_string()),
    }));
    class
}
