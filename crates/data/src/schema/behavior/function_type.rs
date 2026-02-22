#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct FunctionType {
    pub params: Vec<crate::schema::behavior::function_param::FunctionParam>,
    pub results: Vec<crate::schema::core::type_node::Type>,
    pub throws: Option<Box<crate::schema::core::type_node::Type>>,
    pub async_fn: bool,
}
