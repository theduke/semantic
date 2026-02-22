#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct InterfaceMethod {
    pub name: String,
    pub signature: crate::schema::behavior::function_type::FunctionType,
}
