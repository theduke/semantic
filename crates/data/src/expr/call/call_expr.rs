#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct CallExpr {
    pub callee: Callee,
    pub args: Vec<crate::expr::CallArg>,
    pub over: Option<crate::expr::WindowSpec>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Callee {
    Name(Vec<String>),
    Expr(crate::expr::Expr),
}
