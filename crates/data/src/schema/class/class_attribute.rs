#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ClassAttribute {
    pub attribute: crate::schema::attribute::attribute_ref::AttributeRef,
    pub required: bool,
    /// Optional presentation hint for sorting fields in generated UI forms.
    pub ui_order: Option<u32>,
    /// If set, this attribute's value is computed from this expression
    /// at read time rather than stored. The expression must reference
    /// other attributes of the same class via `RefExpr::Identifier("self")`.
    pub computed: Option<crate::expr::Expr>,
    /// Default expression for this field within the class. A literal can be
    /// used to initialize an entity form; other expressions remain available
    /// to consumers that can evaluate them.
    #[facet(default)]
    #[facet(skip_serializing_if = Option::is_none)]
    pub default: Option<crate::expr::Expr>,
    /// Optional extra constraints for this attribute within this class.
    pub constraints: Vec<crate::schema::constraints::constraint::Constraint>,
    pub meta: crate::schema::core::meta::Meta,
}

#[cfg(test)]
mod tests {
    use crate::{
        expr::{Expr, LiteralExpr, RefExpr},
        value::Value,
    };

    use super::ClassAttribute;

    #[test]
    fn default_expression_round_trips_and_absent_default_preserves_old_shape() {
        let mut attribute = crate::filestore::file_class()
            .attributes
            .into_values()
            .next()
            .expect("file class has attributes");
        let without_default = facet_json::to_string(&attribute).unwrap();
        assert!(!without_default.contains("\"default\""));
        assert_eq!(
            facet_json::from_str::<ClassAttribute>(&without_default).unwrap(),
            attribute
        );

        for expression in [
            Expr::Literal(LiteralExpr {
                value: Value::String("draft".to_string()),
            }),
            Expr::Ref(RefExpr::Identifier("source".to_string())),
        ] {
            attribute.default = Some(expression);
            let encoded = facet_json::to_string(&attribute).unwrap();
            assert!(encoded.contains("\"default\""));
            assert_eq!(
                facet_json::from_str::<ClassAttribute>(&encoded).unwrap(),
                attribute
            );
        }
    }
}
