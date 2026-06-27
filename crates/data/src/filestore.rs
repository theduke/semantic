use std::collections::BTreeMap;

use crate::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassType, Constraint, Meta, Migration,
    MigrationDdlOperation, MigrationOperation, Module, NumberType, Package, StringType, Type,
    TypeKind, UIntWidth,
};

pub const PACKAGE_NAME: &str = "semantic.filestore";
pub const MODULE_NAME: &str = "filestore";
pub const INIT_MIGRATION_NAME: &str = "001_init";
pub const GENERIC_METADATA_MIGRATION_NAME: &str = "002_generic_metadata";

pub const FILE_CLASS_ID: &str = "semantic.filestore.file";

pub const TITLE_ATTRIBUTE_ID: &str = "semantic:title";
pub const DESCRIPTION_ATTRIBUTE_ID: &str = "semantic:description";
pub const FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID: &str = "semantic.filestore.file.filestore_locator";
pub const FILE_FILENAME_ATTRIBUTE_ID: &str = "semantic.filestore.file.filename";
pub const FILE_BYTE_SIZE_ATTRIBUTE_ID: &str = "semantic.filestore.file.byte_size";
pub const FILE_MIME_TYPE_ATTRIBUTE_ID: &str = "semantic.filestore.file.mime_type";
pub const FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID: &str =
    "semantic.filestore.file.content_hash_sha256";

pub fn package() -> Package {
    Package {
        name: PACKAGE_NAME.to_string(),
        root: root_module(),
        modules: BTreeMap::new(),
        migrations: vec![init_migration(), generic_metadata_migration()],
        version: None,
        meta: Meta::default(),
    }
}

pub fn root_module() -> Module {
    let attributes = file_attributes()
        .into_iter()
        .map(|attribute| (attribute.id.clone(), attribute))
        .collect();
    let file = file_class();

    Module {
        name: MODULE_NAME.to_string(),
        constants: BTreeMap::new(),
        types: BTreeMap::new(),
        attributes,
        classes: BTreeMap::from([(file.id.clone(), file)]),
        interfaces: BTreeMap::new(),
        contracts: BTreeMap::new(),
        meta: Meta::default(),
    }
}

pub fn init_migration() -> Migration {
    let mut operations = Vec::new();
    for attribute in file_attributes() {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertAttribute { attribute },
        ));
    }
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertClass {
            class: file_class(),
        },
    ));

    Migration {
        module: MODULE_NAME.to_string(),
        name: INIT_MIGRATION_NAME.to_string(),
        description: Some("Initial semantic filestore schema.".to_string()),
        operations,
        meta: Meta::default(),
    }
}

pub fn generic_metadata_migration() -> Migration {
    Migration {
        module: MODULE_NAME.to_string(),
        name: GENERIC_METADATA_MIGRATION_NAME.to_string(),
        description: Some("Attach generic title and description metadata to files.".to_string()),
        operations: vec![
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                attribute: title_attribute(),
            }),
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                attribute: description_attribute(),
            }),
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass {
                class: file_class(),
            }),
        ],
        meta: Meta::default(),
    }
}

pub fn file_attributes() -> Vec<AttributeType> {
    vec![
        title_attribute(),
        description_attribute(),
        attribute(
            FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID,
            "filestore_locator",
            string_type(),
        ),
        attribute(FILE_FILENAME_ATTRIBUTE_ID, "filename", string_type()),
        attribute(FILE_BYTE_SIZE_ATTRIBUTE_ID, "byte_size", uint64_type()),
        attribute(FILE_MIME_TYPE_ATTRIBUTE_ID, "mime_type", string_type()),
        attribute(
            FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID,
            "content_hash_sha256",
            string_type(),
        ),
    ]
}

pub fn file_class() -> ClassType {
    let class_attribute =
        |attribute_id, ui_order| class_attribute_with_ui_order(attribute_id, false, Some(ui_order));

    ClassType {
        id: FILE_CLASS_ID.to_string(),
        name: "File".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes: BTreeMap::from([
            ("title".to_string(), class_attribute(TITLE_ATTRIBUTE_ID, 10)),
            (
                "description".to_string(),
                class_attribute(DESCRIPTION_ATTRIBUTE_ID, 20),
            ),
            (
                "filestore_locator".to_string(),
                class_attribute(FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID, 30),
            ),
            (
                "filename".to_string(),
                class_attribute(FILE_FILENAME_ATTRIBUTE_ID, 40),
            ),
            (
                "byte_size".to_string(),
                class_attribute(FILE_BYTE_SIZE_ATTRIBUTE_ID, 50),
            ),
            (
                "mime_type".to_string(),
                class_attribute(FILE_MIME_TYPE_ATTRIBUTE_ID, 60),
            ),
            (
                "content_hash_sha256".to_string(),
                class_attribute(FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID, 70),
            ),
        ]),
        constraints: Vec::new(),
        meta: meta_with_title("File"),
    }
}

fn title_attribute() -> AttributeType {
    attribute(TITLE_ATTRIBUTE_ID, "title", string_type())
}

fn description_attribute() -> AttributeType {
    attribute(DESCRIPTION_ATTRIBUTE_ID, "description", string_type())
}

fn attribute(id: &str, name: &str, ty: Type) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty,
        constraints: Vec::<Constraint>::new(),
        meta: meta_with_title(title_from_name(name)),
    }
}

fn class_attribute_with_ui_order(
    attribute_id: &str,
    required: bool,
    ui_order: Option<u32>,
) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef {
            id: attribute_id.to_string(),
        },
        required,
        ui_order,
        computed: None,
        constraints: Vec::new(),
        meta: meta_with_title(title_from_attribute_id(attribute_id)),
    }
}

fn string_type() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn uint64_type() -> Type {
    Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64)))
}

fn meta_with_title(title: impl Into<String>) -> Meta {
    Meta {
        title: Some(title.into()),
        ..Meta::default()
    }
}

fn title_from_attribute_id(attribute_id: &str) -> String {
    title_from_name(attribute_id.rsplit('.').next().unwrap_or(attribute_id))
}

fn title_from_name(name: &str) -> String {
    name.split('_')
        .map(title_word)
        .collect::<Vec<String>>()
        .join(" ")
}

fn title_word(word: &str) -> String {
    match word {
        "id" => "ID".to_string(),
        "uri" => "URI".to_string(),
        "url" => "URL".to_string(),
        "urls" => "URLs".to_string(),
        "ui" => "UI".to_string(),
        "mime" => "MIME".to_string(),
        "sha256" => "SHA256".to_string(),
        _ => {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::filestore::{
        FILE_CLASS_ID, FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID, FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID,
        GENERIC_METADATA_MIGRATION_NAME, INIT_MIGRATION_NAME, MODULE_NAME, PACKAGE_NAME,
        TITLE_ATTRIBUTE_ID, package,
    };

    #[test]
    fn package_has_expected_structure() {
        let package = package();

        assert_eq!(package.name, PACKAGE_NAME);
        assert_eq!(package.root.name, MODULE_NAME);
        assert!(package.modules.is_empty());
        assert_eq!(package.migrations.len(), 2);
        assert_eq!(package.migrations[0].name, INIT_MIGRATION_NAME);
        assert_eq!(package.migrations[1].name, GENERIC_METADATA_MIGRATION_NAME);
        assert!(package.root.classes.contains_key(FILE_CLASS_ID));
        assert!(package.root.attributes.contains_key(TITLE_ATTRIBUTE_ID));
        assert!(
            package
                .root
                .attributes
                .contains_key(FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID)
        );
        assert!(
            package
                .root
                .attributes
                .contains_key(FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID)
        );
    }

    #[test]
    fn file_fields_are_optional() {
        let class = super::file_class();
        assert!(
            class
                .attributes
                .values()
                .all(|attribute| !attribute.required)
        );
    }
}
