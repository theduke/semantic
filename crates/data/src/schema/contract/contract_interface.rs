#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ContractInterface {
    pub name: String,
    pub interface: crate::schema::behavior::interface_type::InterfaceType,
    pub meta: crate::schema::core::meta::Meta,
}
