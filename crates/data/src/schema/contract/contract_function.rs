#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ContractFunction {
    pub name: String,
    pub signature: crate::schema::behavior::function_type::FunctionType,
    pub meta: crate::schema::core::meta::Meta,
}
