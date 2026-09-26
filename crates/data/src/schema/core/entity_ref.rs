use crate::schema::core::{type_name::TypeName, type_node::Type};

/// Action applied to an entity that contains a reference when its target is deleted.
#[derive(facet::Facet, Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
pub enum OnDelete {
    /// Reject deletion while a surviving entity still references the target.
    #[default]
    Restrict,
    /// Delete the entity containing the reference.
    Cascade,
}

/// A stored entity-ID reference.
///
/// References always point to the primary ID of an entity in the owning collection.
/// `target: None` accepts an entity of any class while still requiring it to exist.
#[derive(facet::Facet, Clone, Debug, PartialEq)]
pub struct EntityRef {
    /// Target class ID. `None` accepts any entity class.
    #[facet(rename = "name")]
    #[facet(default)]
    pub target: Option<TypeName>,
    /// Retained only so catalogs written before `Named` was introduced decode
    /// through the forward migration boundary.
    #[doc(hidden)]
    #[facet(rename = "args")]
    #[facet(default)]
    #[facet(skip_serializing_if = Vec::is_empty)]
    pub legacy_args: Vec<Type>,
    #[facet(default = OnDelete::Restrict)]
    #[facet(skip_serializing_if = is_restrict)]
    pub on_delete: OnDelete,
}

fn is_restrict(action: &OnDelete) -> bool {
    *action == OnDelete::Restrict
}

impl EntityRef {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: Some(target.into()),
            legacy_args: Vec::new(),
            on_delete: OnDelete::Restrict,
        }
    }

    pub fn any() -> Self {
        Self {
            target: None,
            legacy_args: Vec::new(),
            on_delete: OnDelete::Restrict,
        }
    }

    pub fn with_on_delete(mut self, on_delete: OnDelete) -> Self {
        self.on_delete = on_delete;
        self
    }

    /// Effective target, excluding the historical identity-attribute spelling
    /// that represented an unrestricted reference.
    pub fn target_class(&self) -> Option<&str> {
        self.target
            .as_deref()
            .filter(|target| !matches!(*target, "id" | "semantic:id"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_legacy_reference_shape_with_restrict_default() {
        let reference: EntityRef =
            facet_json::from_str(r#"{"name":"example:Person","args":[]}"#).unwrap();
        assert_eq!(reference.target_class(), Some("example:Person"));
        assert_eq!(reference.on_delete, OnDelete::Restrict);
        assert_eq!(
            facet_json::to_string(&reference).unwrap(),
            r#"{"name":"example:Person"}"#
        );
    }
}
