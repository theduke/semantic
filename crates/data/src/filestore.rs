use std::collections::BTreeMap;

use crate::attr::ATTR_TITLE;
use crate::schema::{
    AttributeRef, AttributeType, BoolType, ClassAttribute, ClassType, Constraint, EnumRepr,
    EnumType, EnumVariant, FloatWidth, Meta, Migration, MigrationDdlOperation, MigrationOperation,
    Module, NumberType, Package, StringType, TemporalType, Type, TypeKind, TypeRef, UIntWidth,
};

pub const PACKAGE_NAME: &str = "semantic.filestore";
pub const MODULE_NAME: &str = "filestore";
pub const INIT_MIGRATION_NAME: &str = "001_init";
pub const GENERIC_METADATA_MIGRATION_NAME: &str = "002_generic_metadata";
pub const FILEKIND_MIGRATION_NAME: &str = "003_filekind";
pub const MEDIA_METADATA_MIGRATION_NAME: &str = "004_media_metadata";
pub const UPLOADED_AT_MIGRATION_NAME: &str = "005_uploaded_at";

pub const FILE_CLASS_ID: &str = "semantic:filestore:file";

pub const ATTR_DESCRIPTION: &str = "semantic:description";
pub const ATTR_PARENT: &str = "semantic:parent";
pub const ATTR_FILE_FILESTORE_LOCATOR: &str = "semantic:filestore:file:filestore_locator";
pub const ATTR_FILE_FILENAME: &str = "semantic:filestore:file:filename";
pub const ATTR_FILE_BYTE_SIZE: &str = "semantic:filestore:file:byte_size";
pub const ATTR_FILE_MIME_TYPE: &str = "semantic:filestore:file:mime_type";
pub const ATTR_FILE_FILEKIND: &str = "semantic:filestore:file:filekind";
pub const ATTR_FILE_CONTENT_HASH_SHA256: &str = "semantic:filestore:file:content_hash_sha256";
pub const ATTR_FILE_UPLOADED_AT: &str = "semantic:file:uploaded_at";
pub const ATTR_FILE_MEDIA_PIXEL_WIDTH: &str = "semantic:filestore:file:media_pixel_width";
pub const ATTR_FILE_MEDIA_PIXEL_HEIGHT: &str = "semantic:filestore:file:media_pixel_height";
pub const ATTR_FILE_MEDIA_DURATION: &str = "semantic:filestore:file:media_duration";
pub const ATTR_FILE_MEDIA_VIDEO_FRAMES_PER_SECOND: &str =
    "semantic:filestore:file:media_video_frames_per_second";
pub const ATTR_FILE_MEDIA_VIDEO_FRAME_COUNT: &str =
    "semantic:filestore:file:media_video_frame_count";
pub const ATTR_FILE_MEDIA_BITRATE: &str = "semantic:filestore:file:media_bitrate";
pub const ATTR_FILE_MEDIA_VIDEO_BITRATE: &str = "semantic:filestore:file:media_video_bitrate";
pub const ATTR_FILE_MEDIA_AUDIO_BITRATE: &str = "semantic:filestore:file:media_audio_bitrate";
pub const ATTR_FILE_MEDIA_HAS_AUDIO: &str = "semantic:filestore:file:media_has_audio";
pub const ATTR_FILE_MEDIA_VIDEO_CODEC: &str = "semantic:filestore:file:media_video_codec";
pub const ATTR_FILE_MEDIA_AUDIO_CODEC: &str = "semantic:filestore:file:media_audio_codec";
pub const ATTR_FILE_MEDIA_AUDIO_CHANNELS: &str = "semantic:filestore:file:media_audio_channels";
pub const ATTR_FILE_MEDIA_AUDIO_SAMPLE_RATE: &str =
    "semantic:filestore:file:media_audio_sample_rate";
pub const ATTR_FILE_MEDIA_CONTAINER_FORMAT: &str = "semantic:filestore:file:media_container_format";

pub const DESCRIPTION_ATTRIBUTE_ID: &str = ATTR_DESCRIPTION;
pub const FILE_FILESTORE_LOCATOR_ATTRIBUTE_ID: &str = ATTR_FILE_FILESTORE_LOCATOR;
pub const FILE_FILENAME_ATTRIBUTE_ID: &str = ATTR_FILE_FILENAME;
pub const FILE_BYTE_SIZE_ATTRIBUTE_ID: &str = ATTR_FILE_BYTE_SIZE;
pub const FILE_MIME_TYPE_ATTRIBUTE_ID: &str = ATTR_FILE_MIME_TYPE;
pub const FILE_CONTENT_HASH_SHA256_ATTRIBUTE_ID: &str = ATTR_FILE_CONTENT_HASH_SHA256;
pub const FILE_UPLOADED_AT_ATTRIBUTE_ID: &str = ATTR_FILE_UPLOADED_AT;
pub const FILE_MEDIA_PIXEL_WIDTH_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_PIXEL_WIDTH;
pub const FILE_MEDIA_PIXEL_HEIGHT_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_PIXEL_HEIGHT;
pub const FILE_MEDIA_DURATION_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_DURATION;
pub const FILE_MEDIA_VIDEO_FRAMES_PER_SECOND_ATTRIBUTE_ID: &str =
    ATTR_FILE_MEDIA_VIDEO_FRAMES_PER_SECOND;
pub const FILE_MEDIA_VIDEO_FRAME_COUNT_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_VIDEO_FRAME_COUNT;
pub const FILE_MEDIA_BITRATE_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_BITRATE;
pub const FILE_MEDIA_VIDEO_BITRATE_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_VIDEO_BITRATE;
pub const FILE_MEDIA_AUDIO_BITRATE_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_AUDIO_BITRATE;
pub const FILE_MEDIA_HAS_AUDIO_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_HAS_AUDIO;
pub const FILE_MEDIA_VIDEO_CODEC_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_VIDEO_CODEC;
pub const FILE_MEDIA_AUDIO_CODEC_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_AUDIO_CODEC;
pub const FILE_MEDIA_AUDIO_CHANNELS_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_AUDIO_CHANNELS;
pub const FILE_MEDIA_AUDIO_SAMPLE_RATE_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_AUDIO_SAMPLE_RATE;
pub const FILE_MEDIA_CONTAINER_FORMAT_ATTRIBUTE_ID: &str = ATTR_FILE_MEDIA_CONTAINER_FORMAT;

pub fn package() -> Package {
    Package {
        name: PACKAGE_NAME.to_string(),
        root: root_module(),
        modules: BTreeMap::new(),
        migrations: vec![
            init_migration(),
            generic_metadata_migration(),
            filekind_migration(),
            media_metadata_migration(),
            uploaded_at_migration(),
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

pub fn media_metadata_migration() -> Migration {
    let mut operations = Vec::new();
    for attribute in media_metadata_migration_attributes() {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertAttribute { attribute },
        ));
    }
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertClass {
            class: media_metadata_migration_file_class(),
        },
    ));

    Migration {
        module: MODULE_NAME.to_string(),
        name: MEDIA_METADATA_MIGRATION_NAME.to_string(),
        description: Some("Add media analysis metadata to files.".to_string()),
        operations,
        meta: Meta::default(),
    }
}

pub fn uploaded_at_migration() -> Migration {
    Migration {
        module: MODULE_NAME.to_string(),
        name: UPLOADED_AT_MIGRATION_NAME.to_string(),
        description: Some("Add file upload timestamps.".to_string()),
        operations: vec![
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                attribute: migration_attribute(
                    ATTR_FILE_UPLOADED_AT,
                    "uploaded_at",
                    migration_datetime_type(),
                    "Uploaded At",
                ),
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
        attribute_with_title(
            ATTR_FILE_UPLOADED_AT,
            "uploaded_at",
            datetime_type(),
            "Uploaded At",
        ),
        attribute(
            ATTR_FILE_MEDIA_PIXEL_WIDTH,
            "media_pixel_width",
            uint64_type(),
        ),
        attribute(
            ATTR_FILE_MEDIA_PIXEL_HEIGHT,
            "media_pixel_height",
            uint64_type(),
        ),
        attribute(ATTR_FILE_MEDIA_DURATION, "media_duration", duration_type()),
        attribute(
            ATTR_FILE_MEDIA_VIDEO_FRAMES_PER_SECOND,
            "media_video_frames_per_second",
            float64_type(),
        ),
        attribute(
            ATTR_FILE_MEDIA_VIDEO_FRAME_COUNT,
            "media_video_frame_count",
            uint64_type(),
        ),
        attribute(ATTR_FILE_MEDIA_BITRATE, "media_bitrate", uint64_type()),
        attribute(
            ATTR_FILE_MEDIA_VIDEO_BITRATE,
            "media_video_bitrate",
            uint64_type(),
        ),
        attribute(
            ATTR_FILE_MEDIA_AUDIO_BITRATE,
            "media_audio_bitrate",
            uint64_type(),
        ),
        attribute(ATTR_FILE_MEDIA_HAS_AUDIO, "media_has_audio", bool_type()),
        attribute(
            ATTR_FILE_MEDIA_VIDEO_CODEC,
            "media_video_codec",
            string_type(),
        ),
        attribute(
            ATTR_FILE_MEDIA_AUDIO_CODEC,
            "media_audio_codec",
            string_type(),
        ),
        attribute(
            ATTR_FILE_MEDIA_AUDIO_CHANNELS,
            "media_audio_channels",
            uint64_type(),
        ),
        attribute(
            ATTR_FILE_MEDIA_AUDIO_SAMPLE_RATE,
            "media_audio_sample_rate",
            uint64_type(),
        ),
        attribute(
            ATTR_FILE_MEDIA_CONTAINER_FORMAT,
            "media_container_format",
            string_type(),
        ),
    ]
}

pub fn file_class() -> ClassType {
    file_class_with_attributes(&[
        ("title", ATTR_TITLE, 10, Some("Title")),
        ("description", ATTR_DESCRIPTION, 20, Some("Description")),
        ("parent", ATTR_PARENT, 30, None),
        (
            "filestore_locator",
            ATTR_FILE_FILESTORE_LOCATOR,
            40,
            Some("File Store Locator"),
        ),
        ("filename", ATTR_FILE_FILENAME, 50, Some("Filename")),
        ("byte_size", ATTR_FILE_BYTE_SIZE, 60, Some("Byte Size")),
        ("mime_type", ATTR_FILE_MIME_TYPE, 70, Some("MIME Type")),
        ("filekind", ATTR_FILE_FILEKIND, 80, Some("File Kind")),
        (
            "content_hash_sha256",
            ATTR_FILE_CONTENT_HASH_SHA256,
            90,
            Some("Content Hash SHA256"),
        ),
        ("media_pixel_width", ATTR_FILE_MEDIA_PIXEL_WIDTH, 100, None),
        (
            "media_pixel_height",
            ATTR_FILE_MEDIA_PIXEL_HEIGHT,
            110,
            None,
        ),
        ("media_duration", ATTR_FILE_MEDIA_DURATION, 120, None),
        (
            "media_video_frames_per_second",
            ATTR_FILE_MEDIA_VIDEO_FRAMES_PER_SECOND,
            130,
            None,
        ),
        (
            "media_video_frame_count",
            ATTR_FILE_MEDIA_VIDEO_FRAME_COUNT,
            140,
            None,
        ),
        ("media_bitrate", ATTR_FILE_MEDIA_BITRATE, 150, None),
        (
            "media_video_bitrate",
            ATTR_FILE_MEDIA_VIDEO_BITRATE,
            160,
            None,
        ),
        (
            "media_audio_bitrate",
            ATTR_FILE_MEDIA_AUDIO_BITRATE,
            170,
            None,
        ),
        ("media_has_audio", ATTR_FILE_MEDIA_HAS_AUDIO, 180, None),
        ("media_video_codec", ATTR_FILE_MEDIA_VIDEO_CODEC, 190, None),
        ("media_audio_codec", ATTR_FILE_MEDIA_AUDIO_CODEC, 200, None),
        (
            "media_audio_channels",
            ATTR_FILE_MEDIA_AUDIO_CHANNELS,
            210,
            None,
        ),
        (
            "media_audio_sample_rate",
            ATTR_FILE_MEDIA_AUDIO_SAMPLE_RATE,
            220,
            None,
        ),
        (
            "media_container_format",
            ATTR_FILE_MEDIA_CONTAINER_FORMAT,
            230,
            None,
        ),
        (
            "uploaded_at",
            ATTR_FILE_UPLOADED_AT,
            240,
            Some("Uploaded At"),
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
            "File Store Locator",
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

fn media_metadata_migration_attributes() -> Vec<AttributeType> {
    vec![
        migration_attribute(
            ATTR_FILE_MEDIA_PIXEL_WIDTH,
            "media_pixel_width",
            migration_uint64_type(),
            "Media Pixel Width",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_PIXEL_HEIGHT,
            "media_pixel_height",
            migration_uint64_type(),
            "Media Pixel Height",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_DURATION,
            "media_duration",
            migration_duration_type(),
            "Media Duration",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_VIDEO_FRAMES_PER_SECOND,
            "media_video_frames_per_second",
            migration_float64_type(),
            "Media Video Frames Per Second",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_VIDEO_FRAME_COUNT,
            "media_video_frame_count",
            migration_uint64_type(),
            "Media Video Frame Count",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_BITRATE,
            "media_bitrate",
            migration_uint64_type(),
            "Media Bitrate",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_VIDEO_BITRATE,
            "media_video_bitrate",
            migration_uint64_type(),
            "Media Video Bitrate",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_AUDIO_BITRATE,
            "media_audio_bitrate",
            migration_uint64_type(),
            "Media Audio Bitrate",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_HAS_AUDIO,
            "media_has_audio",
            migration_bool_type(),
            "Media Has Audio",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_VIDEO_CODEC,
            "media_video_codec",
            migration_string_type(),
            "Media Video Codec",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_AUDIO_CODEC,
            "media_audio_codec",
            migration_string_type(),
            "Media Audio Codec",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_AUDIO_CHANNELS,
            "media_audio_channels",
            migration_uint64_type(),
            "Media Audio Channels",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_AUDIO_SAMPLE_RATE,
            "media_audio_sample_rate",
            migration_uint64_type(),
            "Media Audio Sample Rate",
        ),
        migration_attribute(
            ATTR_FILE_MEDIA_CONTAINER_FORMAT,
            "media_container_format",
            migration_string_type(),
            "Media Container Format",
        ),
    ]
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

fn media_metadata_migration_file_class() -> ClassType {
    let mut class = file_class();
    class.attributes.remove("uploaded_at");
    class
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
        strict_schema: false,
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

fn migration_bool_type() -> Type {
    Type::new(TypeKind::Bool(BoolType))
}

fn migration_float64_type() -> Type {
    Type::new(TypeKind::Number(NumberType::Float(FloatWidth::F64)))
}

fn migration_duration_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::Duration))
}

fn migration_datetime_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::DateTime))
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

fn bool_type() -> Type {
    Type::new(TypeKind::Bool(BoolType))
}

fn float64_type() -> Type {
    Type::new(TypeKind::Number(NumberType::Float(FloatWidth::F64)))
}

fn duration_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::Duration))
}

fn datetime_type() -> Type {
    Type::new(TypeKind::Temporal(TemporalType::DateTime))
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
        "filestore" => "File Store".to_string(),
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
    use crate::attr::ATTR_TITLE;
    use crate::filestore::{
        ATTR_FILE_CONTENT_HASH_SHA256, ATTR_FILE_FILEKIND, ATTR_FILE_FILESTORE_LOCATOR,
        ATTR_FILE_MEDIA_DURATION, ATTR_FILE_MEDIA_PIXEL_WIDTH, ATTR_FILE_UPLOADED_AT, ATTR_PARENT,
        FILE_CLASS_ID, FILEKIND_MIGRATION_NAME, GENERIC_METADATA_MIGRATION_NAME,
        INIT_MIGRATION_NAME, MEDIA_METADATA_MIGRATION_NAME, MODULE_NAME, PACKAGE_NAME,
        UPLOADED_AT_MIGRATION_NAME, package,
    };
    use crate::schema::{
        EnumRepr, FloatWidth, Migration, MigrationDdlOperation, MigrationOperation, NumberType,
        TemporalType, TypeKind,
    };

    #[test]
    fn package_has_expected_structure() {
        let package = package();

        assert_eq!(package.name, PACKAGE_NAME);
        assert_eq!(package.root.name, MODULE_NAME);
        assert!(package.modules.is_empty());
        assert_eq!(package.migrations.len(), 5);
        assert_eq!(package.migrations[0].name, INIT_MIGRATION_NAME);
        assert_eq!(package.migrations[1].name, GENERIC_METADATA_MIGRATION_NAME);
        assert_eq!(package.migrations[2].name, FILEKIND_MIGRATION_NAME);
        assert_eq!(package.migrations[3].name, MEDIA_METADATA_MIGRATION_NAME);
        assert_eq!(package.migrations[4].name, UPLOADED_AT_MIGRATION_NAME);
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
        assert!(
            package
                .root
                .attributes
                .contains_key(ATTR_FILE_MEDIA_PIXEL_WIDTH)
        );
        assert!(
            package
                .root
                .attributes
                .contains_key(ATTR_FILE_MEDIA_DURATION)
        );
        assert!(package.root.attributes.contains_key(ATTR_FILE_UPLOADED_AT));
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
        assert!(!class.attributes.contains_key("media_duration"));
    }

    #[test]
    fn media_metadata_migration_only_adds_media_attributes() {
        let migration = super::media_metadata_migration();
        let ids = migration_upsert_attribute_ids(&migration);

        assert_eq!(ids.len(), 14);
        assert!(ids.iter().all(|id| id.contains(":media_")));

        let class = migration_upsert_class(&migration);
        assert!(class.attributes.contains_key("media_duration"));
        assert!(!class.attributes.contains_key("uploaded_at"));
        assert!(
            class
                .attributes
                .values()
                .all(|attribute| !attribute.required)
        );
    }

    #[test]
    fn uploaded_at_migration_only_adds_uploaded_at_attribute() {
        let migration = super::uploaded_at_migration();

        assert_eq!(
            migration_upsert_attribute_ids(&migration),
            vec![ATTR_FILE_UPLOADED_AT]
        );

        let class = migration_upsert_class(&migration);
        let uploaded_at = class
            .attributes
            .get("uploaded_at")
            .expect("uploaded_at class attribute should exist");
        assert!(!uploaded_at.required);
        assert_eq!(uploaded_at.attribute.id, ATTR_FILE_UPLOADED_AT);
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
    fn all_declared_attributes_have_titles() {
        for attribute in super::file_attributes() {
            assert!(
                attribute
                    .meta
                    .title
                    .as_deref()
                    .is_some_and(|title| !title.is_empty()),
                "{} should have a title",
                attribute.id
            );
        }

        for migration in [
            super::init_migration(),
            super::generic_metadata_migration(),
            super::filekind_migration(),
            super::media_metadata_migration(),
            super::uploaded_at_migration(),
        ] {
            for operation in migration.operations {
                if let MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute {
                    attribute,
                }) = operation
                {
                    assert!(
                        attribute
                            .meta
                            .title
                            .as_deref()
                            .is_some_and(|title| !title.is_empty()),
                        "{} should have a title",
                        attribute.id
                    );
                }
            }
        }
    }

    #[test]
    fn all_declared_class_attributes_have_titles() {
        for class in [
            super::file_class(),
            super::init_migration_file_class(),
            super::generic_metadata_migration_file_class(),
            super::filekind_migration_file_class(),
        ] {
            for (name, attribute) in class.attributes {
                assert!(
                    attribute
                        .meta
                        .title
                        .as_deref()
                        .is_some_and(|title| !title.is_empty()),
                    "{name} should have a title"
                );
            }
        }
    }

    #[test]
    fn file_fields_have_explicit_titles() {
        let expected = [
            ("title", ATTR_TITLE, "Title"),
            ("byte_size", super::ATTR_FILE_BYTE_SIZE, "Byte Size"),
            (
                "content_hash_sha256",
                super::ATTR_FILE_CONTENT_HASH_SHA256,
                "Content Hash SHA256",
            ),
            ("filekind", super::ATTR_FILE_FILEKIND, "File Kind"),
            ("filename", super::ATTR_FILE_FILENAME, "Filename"),
            (
                "filestore_locator",
                super::ATTR_FILE_FILESTORE_LOCATOR,
                "File Store Locator",
            ),
            ("mime_type", super::ATTR_FILE_MIME_TYPE, "MIME Type"),
            ("uploaded_at", super::ATTR_FILE_UPLOADED_AT, "Uploaded At"),
        ];

        let attributes = super::file_attributes()
            .into_iter()
            .map(|attribute| (attribute.id.clone(), attribute))
            .collect::<std::collections::BTreeMap<_, _>>();

        let class = super::file_class();

        for (field_name, attribute_id, title) in expected {
            assert_eq!(
                attributes
                    .get(attribute_id)
                    .and_then(|attribute| attribute.meta.title.as_deref()),
                Some(title),
            );

            assert_eq!(
                class
                    .attributes
                    .get(field_name)
                    .and_then(|attribute| attribute.meta.title.as_deref()),
                Some(title),
            );
        }
    }

    #[test]
    fn generic_metadata_fields_have_explicit_titles() {
        let attributes = super::file_attributes()
            .into_iter()
            .map(|attribute| (attribute.id.clone(), attribute))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            attributes
                .get(ATTR_TITLE)
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

    #[test]
    fn media_metadata_fields_have_expected_types() {
        let attributes = super::file_attributes()
            .into_iter()
            .map(|attribute| (attribute.id.clone(), attribute))
            .collect::<std::collections::BTreeMap<_, _>>();

        assert!(matches!(
            attributes
                .get(super::ATTR_FILE_MEDIA_PIXEL_WIDTH)
                .map(|attribute| &attribute.ty.kind),
            Some(TypeKind::Number(NumberType::UInt(
                crate::schema::UIntWidth::U64
            )))
        ));
        assert!(matches!(
            attributes
                .get(super::ATTR_FILE_MEDIA_DURATION)
                .map(|attribute| &attribute.ty.kind),
            Some(TypeKind::Temporal(TemporalType::Duration))
        ));
        assert!(matches!(
            attributes
                .get(super::ATTR_FILE_MEDIA_HAS_AUDIO)
                .map(|attribute| &attribute.ty.kind),
            Some(TypeKind::Bool(_))
        ));
        assert!(matches!(
            attributes
                .get(super::ATTR_FILE_MEDIA_VIDEO_FRAMES_PER_SECOND)
                .map(|attribute| &attribute.ty.kind),
            Some(TypeKind::Number(NumberType::Float(FloatWidth::F64)))
        ));
        assert!(matches!(
            attributes
                .get(super::ATTR_FILE_UPLOADED_AT)
                .map(|attribute| &attribute.ty.kind),
            Some(TypeKind::Temporal(TemporalType::DateTime))
        ));
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
