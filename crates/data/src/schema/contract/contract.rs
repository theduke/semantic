use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct Contract {
    pub name: String,
    pub constants: BTreeMap<String, crate::schema::contract::contract_constant::ContractConstant>,
    pub types:
        BTreeMap<crate::schema::core::type_name::TypeName, crate::schema::core::type_def::TypeDef>,
    pub functions: BTreeMap<String, crate::schema::contract::contract_function::ContractFunction>,
    pub attributes: BTreeMap<String, crate::schema::attribute::attribute_type::AttributeType>,
    pub classes: BTreeMap<String, crate::schema::class::class_type::ClassType>,
    pub interfaces:
        BTreeMap<String, crate::schema::contract::contract_interface::ContractInterface>,
    pub meta: crate::schema::core::meta::Meta,
}
