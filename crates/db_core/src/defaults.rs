use semantic_data::{
    expr::{Callee, Expr},
    value::{DateTime, Value},
};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultExpressionType {
    DateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DefaultExpressionError {
    #[error("unsupported default expression; expected time.now()")]
    Unsupported,
}

#[derive(Debug, Clone, Copy)]
pub struct DefaultExpressionContext {
    now: DateTime,
}

impl DefaultExpressionContext {
    pub fn now() -> Self {
        Self {
            now: DateTime::now_utc(),
        }
    }

    pub fn at(now: DateTime) -> Self {
        Self { now }
    }
}

pub fn validate_default_expression(
    expr: &Expr,
) -> std::result::Result<DefaultExpressionType, DefaultExpressionError> {
    let Expr::Call(call) = expr else {
        return Err(DefaultExpressionError::Unsupported);
    };
    if !matches!(&call.callee, Callee::Name(name) if name.as_slice() == ["time", "now"])
        || !call.args.is_empty()
        || call.over.is_some()
    {
        return Err(DefaultExpressionError::Unsupported);
    }
    Ok(DefaultExpressionType::DateTime)
}

pub fn evaluate_default_expression(
    expr: &Expr,
    context: &DefaultExpressionContext,
) -> std::result::Result<Value, DefaultExpressionError> {
    match validate_default_expression(expr)? {
        DefaultExpressionType::DateTime => Ok(Value::DateTime(context.now)),
    }
}

#[cfg(test)]
mod tests {
    use semantic_data::expr::{CallExpr, Callee, Expr};
    use semantic_data::value::{DateTime, Value};

    use super::{DefaultExpressionContext, evaluate_default_expression};

    #[test]
    fn time_now_uses_the_write_context_timestamp() {
        let now = DateTime::now_utc();
        let expr = Expr::Call(Box::new(CallExpr {
            callee: Callee::Name(vec!["time".to_string(), "now".to_string()]),
            args: Vec::new(),
            over: None,
        }));

        assert_eq!(
            evaluate_default_expression(&expr, &DefaultExpressionContext::at(now)).unwrap(),
            Value::DateTime(now)
        );
    }
}
