/// The main schema type node: shape + constraints + metadata + annotations.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct Type {
    pub kind: crate::schema::core::type_kind::TypeKind,
    pub constraints: Vec<crate::schema::constraints::constraint::Constraint>,
    pub annotations: Vec<crate::schema::core::annotation::Annotation>,
    pub meta: crate::schema::core::meta::Meta,
}
