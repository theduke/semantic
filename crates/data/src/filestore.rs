use std::collections::BTreeMap;

use crate::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassType, Constraint, EnumRepr, EnumType,
    EnumVariant, Meta, Migration, MigrationDdlOperation, MigrationOperation, Module, NumberType,
    Package, StringType, Type, TypeKind, TypeRef, UIntWidth,
};

pub const PACKAGE_NAME: &str = "semantic.filestore";
pub const MODULE_NAME: &str = "filestore";
pub const INIT_MIGRATION_NAME: &str = "001_init";
pub const GENERIC_METADATA_MIGRATION_NAME: &str = "002_generic_metadata";
pub const FILEKIND_MIGRATION_NAME: &str = "003_filekind";

pub const FILE_CLASS_ID: &str = "semantic:filestore:file";

pub const ATTR_TITLE: &str = "semantic:title";
pub const ATTR_DESCRIPTION: &str = "semantic:description";
pub const ATTR_PARENT: &str = "semantic:parent";
pub const ATTR_FILE_FILESTORE_LOCATOR: &str = "semantic:filestore:file:filestore_locator";
pub const ATTR_FILE_FILENAME: &str = "semantic:filestore:file:filename";
pub const ATTR_FILE_BYTE_SIZE: &str = "semantic:filestore:file:byte_size";
pub const ATTR_FILE_MIME_TYPE: &str = "semantic:filestore:file:mime_type";
pub const ATTR_FILE_FILEKIND: &str = "semantic:filestore:file:filekind";
pub const ATTR_FILE_CONTENT_HASH_SHA256: &str = "semantic:filestore:file:content_hash_sha256";

pub const TITLE_ATTRIBUTE_ID: &str = ATTR_TITLE;
pub const DESCRIPTION_ATTRIBUTE_ID: &str = ATTR_DESCRIPTION;
pub const FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID: &str = ATTR_FILE_FILESTORE_LOCATOR;
pub const FILE_FILENAME_ATTRIBUTE_ID: &str = ATTR_FILE_FILENAME;
pub const FILE_BYTE_SIZE_ATTRIBUTE_ID: &str = ATTR_FILE_BYTE_SIZE;
pub const FILE_MIME_TYPE_ATTRIBUTE_ID: &str = ATTR_FILE_MIME_TYPE;
pub const FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID: &str = ATTR_FILE_CONTENT_HASH_SHA256;

pub fn package() -> Package {
    Package {
        name: PACKAGE_NAME.to_string(),
        root: root_module(),
        modules: BTreeMap::new(),
        migrations: vec![
            init_migration(),
            generic_metadata_migration(),
            filekind_migration(),
        ],
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
    for attribute in init_migration_attributes() {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertAttribute { attribute },
        ));
    }
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertClass {
            class: init_migration_file_class(),
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
    let mut operations = Vec::new();
    for attribute in generic_metadata_migration_attributes() {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertAttribute { attribute },
        ));
    }
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertClass {
            class: generic_metadata_migration_file_class(),
        },
    ));

    Migration {
        module: MODULE_NAME.to_string(),
        name: GENERIC_METADATA_MIGRATION_NAME.to_string(),
        description: Some("Attach generic title and description metadata to files.".to_string()),
        operations,
        meta: Meta::default(),
    }
}

pub fn filekind_migration() -> Migration {
    Migration {
        module: MODULE_NAME.to_string(),
        name: FILEKIND_MIGRATION_NAME.to_string(),
        description: Some("Add filterable file kind metadata to files.".to_string()),
        operations: vec![
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                attribute: filekind_migration_attribute(),
            }),
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass {
                class: filekind_migration_file_class(),
            }),
        ],
        meta: Meta::default(),
    }
}

pub fn file_attributes() -> Vec<AttributeType> {
    vec![
        title_attribute(),
        description_attribute(),
        parent_attribute(),
        attribute(
            ATTR_FILE_FILESTORE_LOCATOR,
            "filestore_locator",
            string_type(),
        ),
        attribute(ATTR_FILE_FILENAME, "filename", string_type()),
        attribute(ATTR_FILE_BYTE_SIZE, "byte_size", uint64_type()),
        attribute(ATTR_FILE_MIME_TYPE, "mime_type", string_type()),
        filekind_attribute(),
        attribute(
            ATTR_FILE_CONTENT_HASH_SHA256,
            "content_hash_sha256",
            string_type(),
        ),
    ]
}

pub fn file_class() -> ClassType {
    file_class_with_attributes(&[
        ("title", ATTR_TITLE, 10, Some("Title")),
        ("description", ATTR_DESCRIPTION, 20, Some("Description")),
        ("parent", ATTR_PARENT, 30, None),
        ("filestore_locator", ATTR_FILE_FILESTORE_LOCATOR, 40, None),
        ("filename", ATTR_FILE_FILENAME, 50, None),
        ("byte_size", ATTR_FILE_BYTE_SIZE, 60, None),
        ("mime_type", ATTR_FILE_MIME_TYPE, 70, None),
        ("filekind", ATTR_FILE_FILEKIND, 80, None),
        (
            "content_hash_sha256",
            ATTR_FILE_CONTENT_HASH_SHA256,
            90,
            None,
        ),
    ])
}

fn init_migration_attributes() -> Vec<AttributeType> {
    vec![
        migration_attribute(
            ATTR_PARENT,
            "parent",
            migration_ref_type(crate::builtin::ATTR_ID),
            "Parent",
        ),
        migration_attribute(
            ATTR_FILE_FILESTORE_LOCATOR,
            "filestore_locator",
            migration_string_type(),
            "Filestore Locator",
        ),
        migration_attribute(
            ATTR_FILE_FILENAME,
            "filename",
            migration_string_type(),
            "Filename",
        ),
        migration_attribute(
            ATTR_FILE_BYTE_SIZE,
            "byte_size",
            migration_uint64_type(),
            "Byte Size",
        ),
        migration_attribute(
            ATTR_FILE_MIME_TYPE,
            "mime_type",
            migration_string_type(),
            "MIME Type",
        ),
        migration_attribute(
            ATTR_FILE_CONTENT_HASH_SHA256,
            "content_hash_sha256",
            migration_string_type(),
            "Content Hash SHA256",
        ),
    ]
}

fn generic_metadata_migration_attributes() -> Vec<AttributeType> {
    vec![
        migration_attribute(ATTR_TITLE, "title", migration_string_type(), "Title"),
        migration_attribute(
            ATTR_DESCRIPTION,
            "description",
            migration_string_type(),
            "Description",
        ),
    ]
}

fn filekind_migration_attribute() -> AttributeType {
    migration_attribute(
        ATTR_FILE_FILEKIND,
        "filekind",
        migration_filekind_type(),
        "File Kind",
    )
}

fn init_migration_file_class() -> ClassType {
    file_class_with_attributes(&[
        ("parent", ATTR_PARENT, 30, None),
        ("filestore_locator", ATTR_FILE_FILESTORE_LOCATOR, 40, None),
        ("filename", ATTR_FILE_FILENAME, 50, None),
        ("byte_size", ATTR_FILE_BYTE_SIZE, 60, None),
        ("mime_type", ATTR_FILE_MIME_TYPE, 70, None),
        (
            "content_hash_sha256",
            ATTR_FILE_CONTENT_HASH_SHA256,
            80,
            None,
        ),
    ])
}

fn generic_metadata_migration_file_class() -> ClassType {
    file_class_with_attributes(&[
        ("title", ATTR_TITLE, 10, Some("Title")),
        ("description", ATTR_DESCRIPTION, 20, Some("Description")),
        ("parent", ATTR_PARENT, 30, None),
        ("filestore_locator", ATTR_FILE_FILESTORE_LOCATOR, 40, None),
        ("filename", ATTR_FILE_FILENAME, 50, None),
        ("byte_size", ATTR_FILE_BYTE_SIZE, 60, None),
        ("mime_type", ATTR_FILE_MIME_TYPE, 70, None),
        (
            "content_hash_sha256",
            ATTR_FILE_CONTENT_HASH_SHA256,
            80,
            None,
        ),
    ])
}

fn filekind_migration_file_class() -> ClassType {
    file_class_with_attributes(&[
        ("title", ATTR_TITLE, 10, Some("Title")),
        ("description", ATTR_DESCRIPTION, 20, Some("Description")),
        ("parent", ATTR_PARENT, 30, None),
        ("filestore_locator", ATTR_FILE_FILESTORE_LOCATOR, 40, None),
        ("filename", ATTR_FILE_FILENAME, 50, None),
        ("byte_size", ATTR_FILE_BYTE_SIZE, 60, None),
        ("mime_type", ATTR_FILE_MIME_TYPE, 70, None),
        ("filekind", ATTR_FILE_FILEKIND, 80, None),
        (
            "content_hash_sha256",
            ATTR_FILE_CONTENT_HASH_SHA256,
            90,
            None,
        ),
    ])
}

fn file_class_with_attributes(attributes: &[(&str, &str, u32, Option<&'static str>)]) -> ClassType {
    let class_attribute =
        |attribute_id, ui_order| class_attribute_with_ui_order(attribute_id, false, Some(ui_order));
    let titled_class_attribute = |attribute_id, ui_order, title| {
        class_attribute_with_ui_order_and_title(attribute_id, false, Some(ui_order), title)
    };
    let attributes = attributes
        .iter()
        .map(|(name, attribute_id, ui_order, title)| {
            let class_attribute = match title {
                Some(title) => titled_class_attribute(attribute_id, *ui_order, *title),
                None => class_attribute(attribute_id, *ui_order),
            };
            ((*name).to_string(), class_attribute)
        })
        .collect();

    ClassType {
        id: FILE_CLASS_ID.to_string(),
        name: "File".to_string(),
        inherits: None,
        extends: Vec::new(),
        attributes,
        constraints: Vec::new(),
        meta: meta_with_title("File"),
    }
}

fn migration_attribute(id: &str, name: &str, ty: Type, title: impl Into<String>) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty,
        constraints: Vec::<Constraint>::new(),
        meta: meta_with_title(title),
    }
}

fn migration_string_type() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn migration_uint64_type() -> Type {
    Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64)))
}

fn migration_ref_type(name: &str) -> Type {
    Type::new(TypeKind::Ref(TypeRef {
        name: name.to_string(),
        args: Vec::new(),
    }))
}

fn migration_filekind_type() -> Type {
    Type::new(TypeKind::Enum(EnumType {
        repr: EnumRepr::String,
        variants: [
            "image", "video", "audio", "text", "document", "archive", "other",
        ]
        .into_iter()
        .map(migration_enum_variant)
        .collect(),
    }))
}

fn migration_enum_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        value: None,
        symbol: None,
        meta: meta_with_title(title_from_name(name)),
    }
}

fn title_attribute() -> AttributeType {
    attribute_with_title(ATTR_TITLE, "title", string_type(), "Title")
}

fn description_attribute() -> AttributeType {
    attribute_with_title(
        ATTR_DESCRIPTION,
        "description",
        string_type(),
        "Description",
    )
}

fn parent_attribute() -> AttributeType {
    attribute(ATTR_PARENT, "parent", ref_type(crate::builtin::ATTR_ID))
}

fn filekind_attribute() -> AttributeType {
    attribute_with_title(ATTR_FILE_FILEKIND, "filekind", filekind_type(), "File Kind")
}

fn attribute(id: &str, name: &str, ty: Type) -> AttributeType {
    attribute_with_title(id, name, ty, title_from_name(name))
}

fn attribute_with_title(id: &str, name: &str, ty: Type, title: impl Into<String>) -> AttributeType {
    AttributeType {
        id: id.to_string(),
        name: name.to_string(),
        ty,
        constraints: Vec::<Constraint>::new(),
        meta: meta_with_title(title),
    }
}

fn class_attribute_with_ui_order(
    attribute_id: &str,
    required: bool,
    ui_order: Option<u32>,
) -> ClassAttribute {
    class_attribute_with_ui_order_and_title(
        attribute_id,
        required,
        ui_order,
        title_from_attribute_id(attribute_id),
    )
}

fn class_attribute_with_ui_order_and_title(
    attribute_id: &str,
    required: bool,
    ui_order: Option<u32>,
    title: impl Into<String>,
) -> ClassAttribute {
    ClassAttribute {
        attribute: AttributeRef {
            id: attribute_id.to_string(),
        },
        required,
        ui_order,
        computed: None,
        constraints: Vec::new(),
        meta: meta_with_title(title),
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

fn filekind_type() -> Type {
    Type::new(TypeKind::Enum(EnumType {
        repr: EnumRepr::String,
        variants: [
            "image", "video", "audio", "text", "document", "archive", "other",
        ]
        .into_iter()
        .map(enum_variant)
        .collect(),
    }))
}

fn enum_variant(name: &str) -> EnumVariant {
    EnumVariant {
        name: name.to_string(),
        value: None,
        symbol: None,
        meta: meta_with_title(title_from_name(name)),
    }
}

fn ref_type(name: &str) -> Type {
    Type::new(TypeKind::Ref(TypeRef {
        name: name.to_string(),
        args: Vec::new(),
    }))
}

fn meta_with_title(title: impl Into<String>) -> Meta {
    Meta {
        title: Some(title.into()),
        ..Meta::default()
    }
}

fn title_from_attribute_id(attribute_id: &str) -> String {
    title_from_name(
        attribute_id
            .rsplit(['.', ':'])
            .next()
            .unwrap_or(attribute_id),
    )
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
        ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILEKIND, ATTR_FILE_FILESTORE_LOCATOR,
        ATTR_PARENT, ATTR_TITLE, FILE_CLASS_ID, FILEKIND_MIGRATION_NAME,
        GENERIC_METADATA_MIGRATION_NAME, INIT_MIGRATION_NAME, MODULE_NAME, PACKAGE_NAME, package,
    };
    use crate::schema::{EnumRepr, Migration, MigrationDdlOperation, MigrationOperation, TypeKind};

    #[test]
    fn package_has_expected_structure() {
        let package = package();

        assert_eq!(package.name, PACKAGE_NAME);
        assert_eq!(package.root.name, MODULE_NAME);
        assert!(package.modules.is_empty());
        assert_eq!(package.migrations.len(), 3);
        assert_eq!(package.migrations[0].name, INIT_MIGRATION_NAME);
        assert_eq!(package.migrations[1].name, GENERIC_METADATA_MIGRATION_NAME);
        assert_eq!(package.migrations[2].name, FILEKIND_MIGRATION_NAME);
        assert!(package.root.classes.contains_key(FILE_CLASS_ID));
        assert!(package.root.attributes.contains_key(ATTR_TITLE));
        assert!(package.root.attributes.contains_key(ATTR_PARENT));
        assert_eq!(ATTR_PARENT, "semantic:parent");
        assert!(
            package
                .root
                .attributes
                .contains_key(ATTR_FILE_FILESTORE_LOCATOR)
        );
        assert!(
            package
                .root
                .attributes
                .contains_key(ATTR_FILE_CONTENT_HASH_SHA256)
        );
        assert!(package.root.attributes.contains_key(ATTR_FILE_FILEKIND));
    }

    #[test]
    fn init_migration_uses_initial_schema_snapshot() {
        let migration = super::init_migration();

        assert_eq!(
            migration_upsert_attribute_ids(&migration),
            vec![
                ATTR_PARENT,
                ATTR_FILE_FILESTORE_LOCATOR,
                super::ATTR_FILE_FILENAME,
                super::ATTR_FILE_BYTE_SIZE,
                super::ATTR_FILE_MIME_TYPE,
                ATTR_FILE_CONTENT_HASH_SHA256,
            ]
        );

        let class = migration_upsert_class(&migration);
        assert_eq!(
            class
                .attributes
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "byte_size",
                "content_hash_sha256",
                "filename",
                "filestore_locator",
                "mime_type",
                "parent",
            ]
        );
        assert!(!class.attributes.contains_key("title"));
        assert!(!class.attributes.contains_key("description"));
        assert!(!class.attributes.contains_key("filekind"));
    }

    #[test]
    fn metadata_migration_does_not_include_later_filekind_schema() {
        let migration = super::generic_metadata_migration();

        assert_eq!(
            migration_upsert_attribute_ids(&migration),
            vec![ATTR_TITLE, super::ATTR_DESCRIPTION]
        );

        let class = migration_upsert_class(&migration);
        assert!(class.attributes.contains_key("title"));
        assert!(class.attributes.contains_key("description"));
        assert!(!class.attributes.contains_key("filekind"));
    }

    #[test]
    fn filekind_migration_only_adds_filekind_attribute() {
        let migration = super::filekind_migration();

        assert_eq!(
            migration_upsert_attribute_ids(&migration),
            vec![ATTR_FILE_FILEKIND]
        );

        let class = migration_upsert_class(&migration);
        assert!(class.attributes.contains_key("filekind"));
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

    #[test]
    fn generic_metadata_fields_have_explicit_titles() {
        let attributes = super::file_attributes()
            .into_iter()
            .map(|attribute| (attribute.id.clone(), attribute))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            attributes
                .get(super::ATTR_TITLE)
                .and_then(|attribute| attribute.meta.title.as_deref()),
            Some("Title")
        );
        assert_eq!(
            attributes
                .get(super::ATTR_DESCRIPTION)
                .and_then(|attribute| attribute.meta.title.as_deref()),
            Some("Description")
        );

        let class = super::file_class();
        assert_eq!(
            class
                .attributes
                .get("title")
                .and_then(|attribute| attribute.meta.title.as_deref()),
            Some("Title")
        );
        assert_eq!(
            class
                .attributes
                .get("description")
                .and_then(|attribute| attribute.meta.title.as_deref()),
            Some("Description")
        );
    }

    #[test]
    fn filekind_is_a_string_enum() {
        let attributes = super::file_attributes()
            .into_iter()
            .map(|attribute| (attribute.id.clone(), attribute))
            .collect::<std::collections::BTreeMap<_, _>>();
        let filekind = attributes
            .get(super::ATTR_FILE_FILEKIND)
            .expect("filekind attribute should exist");

        let TypeKind::Enum(enum_type) = &filekind.ty.kind else {
            panic!("filekind attribute should be an enum");
        };
        assert_eq!(enum_type.repr, EnumRepr::String);
        assert_eq!(
            enum_type
                .variants
                .iter()
                .map(|variant| variant.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "image", "video", "audio", "text", "document", "archive", "other"
            ]
        );

        let class = super::file_class();
        let class_attribute = class
            .attributes
            .get("filekind")
            .expect("filekind class attribute should exist");
        assert!(!class_attribute.required);
        assert_eq!(class_attribute.attribute.id, super::ATTR_FILE_FILEKIND);
    }

    fn migration_upsert_attribute_ids(migration: &Migration) -> Vec<&str> {
        migration
            .operations
            .iter()
            .filter_map(|operation| match operation {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute }) => {
                    Some(attribute.id.as_str())
                }
                _ => None,
            })
            .collect()
    }

    fn migration_upsert_class(migration: &Migration) -> &crate::schema::ClassType {
        migration
            .operations
            .iter()
            .find_map(|operation| match operation {
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }) => {
                    Some(class)
                }
                _ => None,
            })
            .expect("migration should upsert file class")
    }
}
