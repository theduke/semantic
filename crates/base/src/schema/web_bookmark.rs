use std::collections::BTreeMap;

use semantic_data::{
    attr::{ATTR_TITLE, ATTR_URL},
    bundles::directory::{ATTR_CREATED_AT, ATTR_DESCRIPTION, ATTR_UPDATED_AT},
    expr::{CallExpr, Callee, Expr},
    schema::{ClassType, Constraint},
};

use super::common::helpers;

pub const CLASS_ID: &str = "semantic:base:web_bookmark";

/// A saved link with bookmark metadata, rather than metadata about the remote page.
pub fn class() -> ClassType {
    let attributes = [
        ("url", ATTR_URL, true, 10),
        ("title", ATTR_TITLE, false, 20),
        ("description", ATTR_DESCRIPTION, false, 30),
        ("created_at", ATTR_CREATED_AT, false, 40),
        ("updated_at", ATTR_UPDATED_AT, false, 50),
    ]
    .into_iter()
    .map(|(name, id, required, order)| {
        let mut attribute = helpers::class_attribute_with_ui_order(id, required, Some(order));
        if matches!(id, ATTR_CREATED_AT | ATTR_UPDATED_AT) {
            attribute.constraints.push(Constraint::DefaultExpr {
                expr: Expr::Call(Box::new(CallExpr {
                    callee: Callee::Name(vec!["time".to_string(), "now".to_string()]),
                    args: Vec::new(),
                    over: None,
                })),
            });
        }
        (name.to_string(), attribute)
    })
    .collect::<BTreeMap<_, _>>();

    ClassType {
        id: CLASS_ID.to_string(),
        name: "WebBookmark".to_string(),
        inherits: None,
        extends: Vec::new(),
        strict_schema: false,
        attributes,
        constraints: Vec::new(),
        meta: helpers::meta_with_title("WebBookmark"),
    }
}
