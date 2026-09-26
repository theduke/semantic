//! The benchmark schema: an `item` class with twelve attributes that
//! references an `owner` class. Like application entities, both live in one
//! strictly validated polymorphic collection (references resolve within a
//! collection), which carries one index of every kind on item attributes.

use std::collections::BTreeMap;

use semantic_data::query::TextAnalyzer;
use semantic_data::schema::{
    AttributeRef, AttributeType, ClassAttribute, ClassType, EntityRef, Field, FloatWidth,
    IndexKind, IntWidth, ListType, Meta, NumberType, RecordType, StringType, TemporalType, Type,
    TypeKind,
};
use semantic_db_core::catalog::IntegrityMode;
use semantic_db_core::{DdlBatch, DdlCollectionKind, DdlOperation};

/// Collection holding items and their owners.
pub const COLLECTION: &str = "bench_entities";
pub const ITEM_CLASS: &str = "bench:item";
pub const OWNER_CLASS: &str = "bench:owner";

/// Qualified attribute ids; rows store their values under these names.
pub mod attr {
    pub const TITLE: &str = "bench:title";
    pub const BODY: &str = "bench:body";
    pub const KIND: &str = "bench:kind";
    pub const CODE: &str = "bench:code";
    pub const SCORE: &str = "bench:score";
    pub const PRICE: &str = "bench:price";
    pub const RATING: &str = "bench:rating";
    pub const ACTIVE: &str = "bench:active";
    pub const CREATED_AT: &str = "bench:created_at";
    pub const TAGS: &str = "bench:tags";
    pub const DIMENSIONS: &str = "bench:dimensions";
    pub const OWNER: &str = "bench:owner_ref";
    pub const NAME: &str = "bench:name";
    pub const REGION: &str = "bench:region";
    pub const EMAIL: &str = "bench:email";
}

/// Equality index on [`attr::CODE`] (a handful of rows per value).
pub const INDEX_CODE: &str = "by_code";
/// Range index on [`attr::SCORE`] (unique values).
pub const INDEX_SCORE: &str = "by_score";
/// Composite range index on ([`attr::KIND`], [`attr::CREATED_AT`]).
pub const INDEX_KIND_CREATED: &str = "by_kind_created";
/// Full-text index on [`attr::TITLE`] and [`attr::BODY`].
pub const INDEX_SEARCH: &str = "search";

fn string() -> Type {
    Type::new(TypeKind::String(StringType {
        format: None,
        normalization: None,
    }))
}

fn int() -> Type {
    Type::new(TypeKind::Number(NumberType::Int(IntWidth::I64)))
}

fn float() -> Type {
    Type::new(TypeKind::Number(NumberType::Float(FloatWidth::F64)))
}

fn field(ty: Type, required: bool) -> Field {
    Field {
        ty,
        required,
        readonly: false,
        writeonly: false,
        default: None,
        meta: Meta::default(),
    }
}

fn dimensions() -> Type {
    Type::new(TypeKind::Record(RecordType {
        fields: BTreeMap::from([
            ("width".to_string(), field(float(), true)),
            ("height".to_string(), field(float(), true)),
            ("unit".to_string(), field(string(), false)),
        ]),
        open: false,
        additional: None,
        required_order: None,
    }))
}

fn attributes() -> Vec<AttributeType> {
    let attribute = |id: &str, ty: Type| AttributeType {
        id: id.to_string(),
        name: id.rsplit(':').next().unwrap_or(id).to_string(),
        ty,
        constraints: Vec::new(),
        meta: Meta::default(),
    };
    vec![
        attribute(attr::TITLE, string()),
        attribute(attr::BODY, string()),
        attribute(attr::KIND, string()),
        attribute(attr::CODE, string()),
        attribute(attr::SCORE, int()),
        attribute(attr::PRICE, float()),
        attribute(attr::RATING, int()),
        attribute(attr::ACTIVE, Type::new_bool()),
        attribute(
            attr::CREATED_AT,
            Type::new(TypeKind::Temporal(TemporalType::DateTime)),
        ),
        attribute(
            attr::TAGS,
            Type::new(TypeKind::List(ListType {
                items: Box::new(string()),
            })),
        ),
        attribute(attr::DIMENSIONS, dimensions()),
        attribute(
            attr::OWNER,
            Type::new(TypeKind::Ref(EntityRef::new(OWNER_CLASS))),
        ),
        attribute(attr::NAME, string()),
        attribute(attr::REGION, string()),
        attribute(attr::EMAIL, string()),
    ]
}

fn class(id: &str, name: &str, attributes: &[(&str, bool)]) -> ClassType {
    ClassType {
        id: id.to_string(),
        name: name.to_string(),
        inherits: None,
        extends: Vec::new(),
        strict_schema: true,
        creatable_in_ui: None,
        attributes: attributes
            .iter()
            .map(|(attribute, required)| {
                (
                    attribute.to_string(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: attribute.to_string(),
                        },
                        required: *required,
                        ui_order: None,
                        computed: None,
                        constraints: Vec::new(),
                        meta: Meta::default(),
                    },
                )
            })
            .collect(),
        constraints: Vec::new(),
        meta: Meta::default(),
    }
}

fn index(name: &str, fields: &[&str], kind: IndexKind) -> DdlOperation {
    DdlOperation::UpsertIndex {
        name: name.to_string(),
        collection: COLLECTION.to_string(),
        field: fields[0].to_string(),
        unique: false,
        kind,
        extra_fields: fields[1..].iter().map(ToString::to_string).collect(),
        predicate: None,
        analyzer: TextAnalyzer::default(),
    }
}

/// DDL creating the attributes, classes, collections and indexes.
pub fn ddl() -> DdlBatch {
    let ddl = attributes()
        .into_iter()
        .fold(DdlBatch::new(), |ddl, attribute| {
            ddl.with_op(DdlOperation::UpsertAttribute { attribute })
        });
    let owner = class(
        OWNER_CLASS,
        "BenchOwner",
        &[
            (attr::NAME, true),
            (attr::REGION, true),
            (attr::EMAIL, false),
        ],
    );
    let item = class(
        ITEM_CLASS,
        "BenchItem",
        &[
            (attr::TITLE, true),
            (attr::BODY, false),
            (attr::KIND, true),
            (attr::CODE, true),
            (attr::SCORE, true),
            (attr::PRICE, true),
            (attr::RATING, false),
            (attr::ACTIVE, true),
            (attr::CREATED_AT, true),
            (attr::TAGS, false),
            (attr::DIMENSIONS, false),
            (attr::OWNER, true),
        ],
    );
    ddl.with_op(DdlOperation::UpsertClass { class: owner })
        .with_op(DdlOperation::UpsertClass { class: item })
        .with_op(DdlOperation::UpsertCollection {
            name: COLLECTION.to_string(),
            kind: DdlCollectionKind::Polymorphic,
            integrity_mode: IntegrityMode::StrictRegisteredSchema,
        })
        .with_op(index(INDEX_CODE, &[attr::CODE], IndexKind::Equality))
        .with_op(index(INDEX_SCORE, &[attr::SCORE], IndexKind::Range))
        .with_op(index(
            INDEX_KIND_CREATED,
            &[attr::KIND, attr::CREATED_AT],
            IndexKind::Range,
        ))
        .with_op(index(
            INDEX_SEARCH,
            &[attr::TITLE, attr::BODY],
            IndexKind::FullText,
        ))
}
