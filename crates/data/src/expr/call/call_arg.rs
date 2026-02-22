#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum CallArg {
    Positional(crate::expr::Expr),
    Named {
        name: String,
        value: crate::expr::Expr,
    },
}
