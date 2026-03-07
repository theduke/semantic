#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum Query {
    Select(Select),
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct SubqueryExpr {
    pub query: Box<Select>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct Select {
    pub projection: Vec<SelectExpr>,
    pub from: Vec<FromItem>,
    pub selection: Option<crate::expr::Expr>,
    pub group_by: Vec<crate::expr::Expr>,
    pub having: Option<crate::expr::Expr>,
    pub order_by: Vec<OrderByExpr>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub distinct: bool,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct SelectExpr {
    pub expr: crate::expr::Expr,
    pub alias: Option<String>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum FromItem {
    Table {
        name: Vec<String>,
        alias: Option<String>,
    },
    Subquery {
        query: Box<Select>,
        alias: String,
    },
    Join(JoinExpr),
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct JoinExpr {
    pub left: Box<FromItem>,
    pub right: Box<FromItem>,
    pub kind: JoinKind,
    pub on: Option<crate::expr::Expr>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum JoinKind {
    Inner,
    Left,
    Right,
    Full,
    Cross,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct OrderByExpr {
    pub expr: crate::expr::Expr,
    pub direction: SortDirection,
    pub nulls: Option<NullsOrder>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum NullsOrder {
    First,
    Last,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct WindowSpec {
    pub partition_by: Vec<crate::expr::Expr>,
    pub order_by: Vec<OrderByExpr>,
    pub frame: Option<WindowFrame>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct WindowFrame {
    pub units: WindowFrameUnits,
    pub start: WindowFrameBound,
    pub end: Option<WindowFrameBound>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum WindowFrameUnits {
    Rows,
    Range,
    Groups,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum WindowFrameBound {
    UnboundedPreceding,
    Preceding(u64),
    CurrentRow,
    Following(u64),
    UnboundedFollowing,
}
