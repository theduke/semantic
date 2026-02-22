#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct HandleType {
    pub interface: crate::schema::core::type_ref::TypeRef,
    pub mode: crate::schema::handle::handle_mode::HandleMode,
}
