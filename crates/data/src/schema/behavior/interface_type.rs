#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct InterfaceType {
    pub methods: Vec<crate::schema::behavior::interface_method::InterfaceMethod>,
}
