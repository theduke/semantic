use std::collections::BTreeMap;

/// A module groups constants, types, interfaces, and contracts.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct Module {
    pub name: String,
    pub constants: BTreeMap<String, crate::schema::contract::contract_constant::ContractConstant>,
    pub types:
        BTreeMap<crate::schema::core::type_name::TypeName, crate::schema::core::type_def::TypeDef>,
    pub attributes: BTreeMap<String, crate::schema::attribute::attribute_type::AttributeType>,
    pub classes: BTreeMap<String, crate::schema::class::class_type::ClassType>,
    pub interfaces: BTreeMap<String, crate::schema::behavior::interface_type::InterfaceType>,
    pub contracts: BTreeMap<String, crate::schema::contract::contract::Contract>,
    pub meta: crate::schema::core::meta::Meta,
}
