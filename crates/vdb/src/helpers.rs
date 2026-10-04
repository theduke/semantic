use semantic_data::Value;
use semantic_data::query::{BinaryOp, Expr, Operand};
use semantic_data::value::{FieldPath, PathSegment};

use crate::FilterSupport;

/// Classify field/literal comparisons and non-negated literal IN lists.
/// Complex expressions remain unsupported, even if they contain comparisons.
pub fn classify_simple(
    filters: &[Expr],
    supported: impl Fn(&FieldPath, BinaryOp) -> bool,
) -> Vec<FilterSupport> {
    filters
        .iter()
        .map(|filter| {
            let candidate = match filter {
                Expr::Binary { op, left, right } => match (&**left, &**right) {
                    (Expr::Operand(Operand::Field(field)), Expr::Operand(Operand::Literal(_))) => {
                        Some((field, *op))
                    }
                    _ => None,
                },
                Expr::InList {
                    expr,
                    list,
                    negated: false,
                } if list
                    .iter()
                    .all(|item| matches!(item, Expr::Operand(Operand::Literal(_)))) =>
                {
                    match &**expr {
                        Expr::Operand(Operand::Field(field)) => Some((field, BinaryOp::In)),
                        _ => None,
                    }
                }
                _ => None,
            };
            if candidate.is_some_and(|(field, op)| supported(field, op)) {
                FilterSupport::Exact
            } else {
                FilterSupport::Unsupported
            }
        })
        .collect()
}

/// Extract the literal from a single-field equality, such as `type = 'x:C'`.
pub fn simple_eq_value<'a>(filters: &'a [Expr], field: &str) -> Option<&'a Value> {
    filters.iter().find_map(|filter| match filter {
        Expr::Binary {
            op: BinaryOp::Eq,
            left,
            right,
        } => match (&**left, &**right) {
            (Expr::Operand(Operand::Field(path)), Expr::Operand(Operand::Literal(value)))
                if matches!(path.segments(), [PathSegment::Field(name)] if name == field) =>
            {
                Some(value)
            }
            _ => None,
        },
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str) -> Expr {
        Expr::Operand(Operand::Field(FieldPath::from_fields([name])))
    }

    fn literal(value: &str) -> Expr {
        Expr::Operand(Operand::Literal(Value::String(value.into())))
    }

    #[test]
    fn classify_comparisons_and_literal_lists() {
        let filters = vec![
            Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(field("id")),
                right: Box::new(literal("one")),
            },
            Expr::InList {
                expr: Box::new(field("id")),
                list: vec![literal("one"), literal("two")],
                negated: false,
            },
            Expr::InList {
                expr: Box::new(field("id")),
                list: vec![field("other")],
                negated: false,
            },
            Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(field("id")),
                right: Box::new(Expr::Operand(Operand::Parameter("key".into()))),
            },
            Expr::InList {
                expr: Box::new(field("id")),
                list: vec![literal("one")],
                negated: true,
            },
        ];
        assert_eq!(
            classify_simple(&filters, |path, op| {
                *path == FieldPath::from_fields(["id"]) && matches!(op, BinaryOp::Eq | BinaryOp::In)
            }),
            vec![
                FilterSupport::Exact,
                FilterSupport::Exact,
                FilterSupport::Unsupported,
                FilterSupport::Unsupported,
                FilterSupport::Unsupported,
            ]
        );
        assert_eq!(
            classify_simple(&filters, |_, _| false),
            vec![FilterSupport::Unsupported; 5]
        );
    }

    #[test]
    fn routing_equality_requires_an_exact_field_path() {
        let filters = vec![Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(field("type")),
            right: Box::new(literal("tiny:Item")),
        }];
        assert_eq!(
            simple_eq_value(&filters, "type"),
            Some(&Value::String("tiny:Item".into()))
        );
        assert_eq!(simple_eq_value(&filters, "id"), None);
        let nested = vec![Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Operand(Operand::Field(FieldPath::from_fields([
                "a", "type",
            ])))),
            right: Box::new(literal("tiny:Item")),
        }];
        assert_eq!(simple_eq_value(&nested, "type"), None);
    }
}
