use super::*;

ast_enum!(expr::BinaryOperator, "semantic:expr:BinaryOperator", {
    Add => "add",
    Sub => "sub",
    Mul => "mul",
    Div => "div",
    Mod => "mod",
    Pow => "pow",
    Eq => "eq",
    Ne => "ne",
    Lt => "lt",
    Le => "le",
    Gt => "gt",
    Ge => "ge",
    And => "and",
    Or => "or",
    BitAnd => "bit_and",
    BitOr => "bit_or",
    BitXor => "bit_xor",
    ShiftLeft => "shift_left",
    ShiftRight => "shift_right",
    Concat => "concat",
    Coalesce => "coalesce",
});

ast_variant!(expr::CallArg, "semantic:expr:CallArg", {
    Positional => "positional" (inner: expr::Expr),
    Named => "named" {
        name: String => "name" [required],
        value: expr::Expr => "value" [required],
    },
});

ast_record!(expr::CallExpr, "semantic:expr:CallExpr", {
    callee: expr::Callee => "callee" [required],
    args: Vec<expr::CallArg> => "args" [required],
    over: Option<expr::WindowSpec> => "over" [default None],
});

ast_variant!(expr::Callee, "semantic:expr:Callee", {
    Name => "name" (inner: Vec<String>),
    Expr => "expr" (inner: expr::Expr),
});

ast_record!(expr::CaseBranch, "semantic:expr:CaseBranch", {
    when: expr::Expr => "when" [required],
    then_expr: expr::Expr => "then_expr" [required],
});

ast_record!(expr::CaseExpr, "semantic:expr:CaseExpr", {
    operand: Option<expr::Expr> => "operand" [default None],
    branches: Vec<expr::CaseBranch> => "branches" [required],
    else_expr: Option<expr::Expr> => "else_expr" [default None],
});

ast_record!(expr::CastExpr, "semantic:expr:CastExpr", {
    expr: expr::Expr => "expr" [required],
    to: schema::Type => "to" [required],
    safe: bool => "safe" [required],
});

ast_record!(expr::Delete, "semantic:expr:Delete", {
    from: Vec<String> => "from" [required],
    selection: Option<expr::Expr> => "selection" [default None],
    returning: Vec<expr::SelectExpr> => "returning" [required],
    limit: Option<u64> => "limit" [default None],
});

ast_record!(expr::ExistsExpr, "semantic:expr:ExistsExpr", {
    query: Box<expr::Select> => "query" [required],
    negated: bool => "negated" [required],
});

ast_variant!(expr::Expr, "semantic:expr:Expr", {
    Literal => "literal" (inner: expr::LiteralExpr),
    Ref => "ref" (inner: expr::RefExpr),
    Unary => "unary" (inner: Box<expr::UnaryExpr>),
    Binary => "binary" (inner: Box<expr::BinaryExpr>),
    Call => "call" (inner: Box<expr::CallExpr>),
    FieldAccess => "field_access" (inner: Box<expr::FieldAccessExpr>),
    IndexAccess => "index_access" (inner: Box<expr::IndexAccessExpr>),
    Tuple => "tuple" (inner: expr::TupleExpr),
    List => "list" (inner: expr::ListExpr),
    Map => "map" (inner: expr::MapExpr),
    Cast => "cast" (inner: Box<expr::CastExpr>),
    If => "if" (inner: Box<expr::IfExpr>),
    Case => "case" (inner: Box<expr::CaseExpr>),
    Let => "let" (inner: Box<expr::LetExpr>),
    Lambda => "lambda" (inner: Box<expr::LambdaExpr>),
    Between => "between" (inner: Box<expr::BetweenExpr>),
    In => "in" (inner: Box<expr::InExpr>),
    Like => "like" (inner: Box<expr::LikeExpr>),
    Regex => "regex" (inner: Box<expr::RegexExpr>),
    IsNull => "is_null" (inner: Box<expr::IsNullExpr>),
    Exists => "exists" (inner: Box<expr::ExistsExpr>),
    Query => "query" (inner: Box<expr::Query>),
    Subquery => "subquery" (inner: Box<expr::SubqueryExpr>),
});

ast_record!(expr::FieldAccessExpr, "semantic:expr:FieldAccessExpr", {
    target: expr::Expr => "target" [required],
    field: String => "field" [required],
});

ast_variant!(expr::FromItem, "semantic:expr:FromItem", {
    Table => "table" {
        name: Vec<String> => "name" [required],
        alias: Option<String> => "alias" [default None],
    },
    Subquery => "subquery" {
        query: Box<expr::Select> => "query" [required],
        alias: String => "alias" [required],
    },
    Join => "join" (inner: expr::JoinExpr),
});

ast_record!(expr::IfExpr, "semantic:expr:IfExpr", {
    condition: expr::Expr => "condition" [required],
    then_expr: expr::Expr => "then_expr" [required],
    else_expr: expr::Expr => "else_expr" [required],
});

ast_record!(expr::InExpr, "semantic:expr:InExpr", {
    value: expr::Expr => "value" [required],
    set: expr::InSet => "set" [required],
    negated: bool => "negated" [required],
});

ast_variant!(expr::InSet, "semantic:expr:InSet", {
    Exprs => "exprs" (inner: Vec<expr::Expr>),
    Subquery => "subquery" (inner: Box<expr::Select>),
});

ast_record!(expr::IndexAccessExpr, "semantic:expr:IndexAccessExpr", {
    target: expr::Expr => "target" [required],
    index: expr::Expr => "index" [required],
});

ast_record!(expr::IsNullExpr, "semantic:expr:IsNullExpr", {
    value: expr::Expr => "value" [required],
    negated: bool => "negated" [required],
});

ast_record!(expr::JoinExpr, "semantic:expr:JoinExpr", {
    left: Box<expr::FromItem> => "left" [required],
    right: Box<expr::FromItem> => "right" [required],
    kind: expr::JoinKind => "kind" [required],
    on: Option<expr::Expr> => "on" [default None],
});

ast_enum!(expr::JoinKind, "semantic:expr:JoinKind", {
    Inner => "inner",
    Left => "left",
    Right => "right",
    Full => "full",
    Cross => "cross",
});

ast_record!(expr::LambdaExpr, "semantic:expr:LambdaExpr", {
    params: Vec<expr::LambdaParam> => "params" [required],
    body: expr::Expr => "body" [required],
});

ast_record!(expr::LambdaParam, "semantic:expr:LambdaParam", {
    name: String => "name" [required],
    ty: Option<schema::Type> => "ty" [default None],
});

ast_record!(expr::LetBinding, "semantic:expr:LetBinding", {
    name: String => "name" [required],
    value: expr::Expr => "value" [required],
});

ast_record!(expr::LetExpr, "semantic:expr:LetExpr", {
    bindings: Vec<expr::LetBinding> => "bindings" [required],
    body: expr::Expr => "body" [required],
});

ast_record!(expr::LikeExpr, "semantic:expr:LikeExpr", {
    kind: expr::LikeKind => "kind" [required],
    value: expr::Expr => "value" [required],
    pattern: expr::Expr => "pattern" [required],
    escape: Option<expr::Expr> => "escape" [default None],
    case_insensitive: bool => "case_insensitive" [required],
    negated: bool => "negated" [required],
});

ast_enum!(expr::LikeKind, "semantic:expr:LikeKind", {
    Like => "like",
    SimilarTo => "similar_to",
});

ast_record!(expr::ListExpr, "semantic:expr:ListExpr", {
    items: Vec<expr::Expr> => "items" [required],
});

ast_record!(expr::LiteralExpr, "semantic:expr:LiteralExpr", {
    value: value::Value => "value" [required],
});

ast_record!(expr::MapEntryExpr, "semantic:expr:MapEntryExpr", {
    key: expr::Expr => "key" [required],
    value: expr::Expr => "value" [required],
});

ast_record!(expr::MapExpr, "semantic:expr:MapExpr", {
    entries: Vec<expr::MapEntryExpr> => "entries" [required],
});

ast_enum!(expr::NullsOrder, "semantic:expr:NullsOrder", {
    First => "first",
    Last => "last",
});

ast_record!(expr::OrderByExpr, "semantic:expr:OrderByExpr", {
    expr: expr::Expr => "expr" [required],
    direction: query::SortDirection => "direction" [required],
    nulls: Option<expr::NullsOrder> => "nulls" [default None],
});

ast_record!(expr::ParameterRef, "semantic:expr:ParameterRef", {
    name: String => "name" [required],
});

ast_variant!(expr::Query, "semantic:expr:Query", {
    Select => "select" (inner: expr::Select),
    Update => "update" (inner: expr::Update),
    Delete => "delete" (inner: expr::Delete),
});

ast_variant!(expr::RefExpr, "semantic:expr:RefExpr", {
    Identifier => "identifier" (inner: String),
    Qualified => "qualified" (inner: Vec<String>),
    Parameter => "parameter" (inner: expr::ParameterRef),
    Variable => "variable" (inner: expr::VariableRef),
    CurrentRow => "current_row",
});

ast_record!(expr::RegexExpr, "semantic:expr:RegexExpr", {
    value: expr::Expr => "value" [required],
    pattern: expr::Expr => "pattern" [required],
    case_insensitive: bool => "case_insensitive" [required],
    negated: bool => "negated" [required],
});

ast_record!(expr::Select, "semantic:expr:Select", {
    projection: Vec<expr::SelectExpr> => "projection" [required],
    from: Vec<expr::FromItem> => "from" [required],
    selection: Option<expr::Expr> => "selection" [default None],
    group_by: Vec<expr::Expr> => "group_by" [required],
    having: Option<expr::Expr> => "having" [default None],
    order_by: Vec<expr::OrderByExpr> => "order_by" [required],
    limit: Option<u64> => "limit" [default None],
    offset: Option<u64> => "offset" [default None],
    distinct: bool => "distinct" [required],
});

ast_record!(expr::SelectExpr, "semantic:expr:SelectExpr", {
    expr: expr::Expr => "expr" [required],
    alias: Option<String> => "alias" [default None],
});

ast_record!(expr::SubqueryExpr, "semantic:expr:SubqueryExpr", {
    query: Box<expr::Select> => "query" [required],
});

ast_record!(expr::TupleExpr, "semantic:expr:TupleExpr", {
    items: Vec<expr::Expr> => "items" [required],
});

ast_record!(expr::UnaryExpr, "semantic:expr:UnaryExpr", {
    op: expr::UnaryOperator => "op" [required],
    operand: expr::Expr => "operand" [required],
});

ast_enum!(expr::UnaryOperator, "semantic:expr:UnaryOperator", {
    Plus => "plus",
    Minus => "minus",
    Not => "not",
    BitNot => "bit_not",
});

ast_record!(expr::Update, "semantic:expr:Update", {
    table: Vec<String> => "table" [required],
    assignments: Vec<expr::UpdateAssignment> => "assignments" [required],
    selection: Option<expr::Expr> => "selection" [default None],
    returning: Vec<expr::SelectExpr> => "returning" [required],
    limit: Option<u64> => "limit" [default None],
});

ast_record!(expr::UpdateAssignment, "semantic:expr:UpdateAssignment", {
    target: Vec<String> => "target" [required],
    value: expr::Expr => "value" [required],
});

ast_record!(expr::VariableRef, "semantic:expr:VariableRef", {
    name: String => "name" [required],
});

ast_record!(expr::WindowFrame, "semantic:expr:WindowFrame", {
    units: expr::WindowFrameUnits => "units" [required],
    start: expr::WindowFrameBound => "start" [required],
    end: Option<expr::WindowFrameBound> => "end" [default None],
});

ast_variant!(expr::WindowFrameBound, "semantic:expr:WindowFrameBound", {
    UnboundedPreceding => "unbounded_preceding",
    Preceding => "preceding" (inner: u64),
    CurrentRow => "current_row",
    Following => "following" (inner: u64),
    UnboundedFollowing => "unbounded_following",
});

ast_enum!(expr::WindowFrameUnits, "semantic:expr:WindowFrameUnits", {
    Rows => "rows",
    Range => "range",
    Groups => "groups",
});

ast_record!(expr::WindowSpec, "semantic:expr:WindowSpec", {
    partition_by: Vec<expr::Expr> => "partition_by" [required],
    order_by: Vec<expr::OrderByExpr> => "order_by" [required],
    frame: Option<expr::WindowFrame> => "frame" [default None],
});

ast_record!(expr::BetweenExpr, "semantic:expr:BetweenExpr", {
    value: expr::Expr => "value" [required],
    lower: expr::Expr => "lower" [required],
    upper: expr::Expr => "upper" [required],
    negated: bool => "negated" [required],
});

ast_record!(expr::BinaryExpr, "semantic:expr:BinaryExpr", {
    op: expr::BinaryOperator => "op" [required],
    left: expr::Expr => "left" [required],
    right: expr::Expr => "right" [required],
});
