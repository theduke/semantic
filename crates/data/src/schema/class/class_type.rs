use std::collections::BTreeMap;

#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct ClassType {
    /// Globally unique class identifier.
    pub id: String,
    pub name: String,
    /// Exactly one inherited base class, if any.
    pub inherits: Option<crate::schema::class::class_ref::ClassRef>,
    /// Additional extension classes (mixin style).
    pub extends: Vec<crate::schema::class::class_ref::ClassRef>,
    /// Declared attributes that make up this class.
    pub attributes: BTreeMap<String, crate::schema::class::class_attribute::ClassAttribute>,
    /// Class-level constraints, including multi-field constraints.
    pub constraints: Vec<crate::schema::class::class_constraint::ClassConstraint>,
    pub meta: crate::schema::core::meta::Meta,
}
