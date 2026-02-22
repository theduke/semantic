#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum RefExpr {
    Identifier(String),
    Qualified(Vec<String>),
    Parameter(ParameterRef),
    Variable(VariableRef),
    CurrentRow,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ParameterRef {
    pub name: String,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VariableRef {
    pub name: String,
}
