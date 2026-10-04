use std::collections::BTreeSet;

use semantic_data::{Object, Value};
use semantic_db_core::catalog::{Catalog, LocalClassId, is_special_builtin_field};
use semantic_db_core::normalize_object_for_collection;
use semantic_db_core::{WriteSettings, validate_stored_object_with_settings};

use crate::{DatabaseDescriptor, VdbError};
/// Validate the entity's structure, canonical class attributes and value types
/// with the existing transaction-free normalizer and stored-value validator.
/// Required fields and declared value constraints are checked; defaults are
/// not filled into remote entities. Foreign-key existence is
/// intentionally not checked: virtual rows do not belong to persisted storage.
/// `collection` identifies the virtual collection in the prepared overlay.
pub fn validate_entity(
    entity: &Object,
    descriptor: &DatabaseDescriptor,
    overlay: &Catalog,
    collection: &str,
) -> Result<(), VdbError> {
    let invalid = |message: String| VdbError {
        code: "invalid_entity".into(),
        message,
    };
    if !matches!(entity.get("id"), Some(Value::String(id)) if !id.is_empty()) {
        return Err(invalid("id must be a non-empty string".into()));
    }
    match entity.get("type") {
        None if descriptor.allow_untyped => return Ok(()),
        None => return Err(invalid("untyped entities are not allowed".into())),
        Some(Value::String(class)) if descriptor.schema.class_ids().any(|id| id == class) => {
            let class_lid = overlay
                .class_id(class)
                .ok_or_else(|| invalid(format!("class '{class}' is absent from the overlay")))?;
            let mut attributes = BTreeSet::new();
            collect_attributes(overlay, class_lid, &mut BTreeSet::new(), &mut attributes);
            for key in entity.keys() {
                if !is_special_builtin_field(key) && !attributes.contains(key) {
                    return Err(invalid(format!(
                        "attribute '{key}' does not belong to class '{class}'"
                    )));
                }
            }
        }
        Some(_) => return Err(invalid("type must name an exposed class".into())),
    }
    let collection = overlay
        .collection_by_name(collection)
        .ok_or_else(|| invalid("virtual collection is absent from the overlay".into()))?;
    let mut normalized = entity.clone();
    normalize_object_for_collection(overlay, collection, &mut normalized)
        .map_err(|error| invalid(error.to_string()))?;
    validate_values(entity, overlay, &collection.name)
}

fn validate_values(entity: &Object, overlay: &Catalog, collection: &str) -> Result<(), VdbError> {
    let owner = (
        collection.to_owned(),
        entity
            .get("id")
            .and_then(Value::as_str)
            .expect("validated id")
            .to_owned(),
    );
    validate_stored_object_with_settings(
        overlay,
        &owner,
        entity,
        |_| Ok(None),
        WriteSettings {
            validate_foreign_keys: false,
        },
    )
    .map(|_| ())
    .map_err(|error| VdbError {
        code: "invalid_entity".into(),
        message: error.to_string(),
    })
}

fn collect_attributes(
    catalog: &Catalog,
    lid: LocalClassId,
    seen: &mut BTreeSet<LocalClassId>,
    attributes: &mut BTreeSet<String>,
) {
    if !seen.insert(lid) {
        return;
    }
    let Some(class) = catalog.class_by_lid(lid) else {
        return;
    };
    if let Some(parent) = &class.class.inherits
        && let Some(lid) = catalog.class_id(&parent.id)
    {
        collect_attributes(catalog, lid, seen, attributes);
    }
    for extension in &class.class.extends {
        if let Some(lid) = catalog.class_id(&extension.id) {
            collect_attributes(catalog, lid, seen, attributes);
        }
    }
    attributes.extend(
        class
            .class
            .attributes
            .values()
            .map(|attribute| attribute.attribute.id.clone()),
    );
}
