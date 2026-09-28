//! Random schemas of the differential test: two polymorphic collections
//! with a random subset of equality, range, composite, partial, full-text
//! and unique indexes, plus three classes with references stored in the
//! default collection.

use std::collections::BTreeMap;

use semantic_data::query::{BinaryOp, Expr, Operand, TextAnalyzer};
use semantic_data::schema::attribute::{
    attribute_ref::AttributeRef, attribute_type::AttributeType,
};
use semantic_data::schema::{
    ClassAttribute, ClassType, EntityRef, IndexKind, Meta, StringType, Type, TypeKind,
};
use semantic_data::value::{FieldPath, Value};
use semantic_db_core::catalog::IntegrityMode;
use semantic_db_core::{DdlBatch, DdlCollectionKind, DdlOperation};

use crate::rng::TestRng;

/// Collections of plain (untyped) rows.
pub(super) const PLAIN: [&str; 2] = ["diff_a", "diff_b"];
/// Collection of the class rows.
pub(super) const CLASSES: &str = semantic_db_core::DEFAULT_COLLECTION;
pub(super) const PERSON: &str = "diff:Person";
pub(super) const DOC: &str = "diff:Doc";
pub(super) const NOTE: &str = "diff:Note";
/// Reference from a doc to a person.
pub(super) const AUTHOR: &str = "diff:author";
/// Reference from a note to a doc.
pub(super) const SUBJECT: &str = "diff:subject";
pub(super) const LABEL: &str = "diff:label";

/// One index of the schema.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct IndexSpec {
    pub name: String,
    pub collection: String,
    pub kind: IndexKind,
    pub fields: Vec<String>,
    pub unique: bool,
    pub predicate: Option<Expr>,
}

impl IndexSpec {
    fn new(collection: &str, suffix: &str, kind: IndexKind, fields: &[&str]) -> Self {
        Self {
            name: format!("{}_{suffix}", collection.replace([':', '.'], "_")),
            collection: collection.to_string(),
            kind,
            fields: fields.iter().map(ToString::to_string).collect(),
            unique: false,
            predicate: None,
        }
    }

    pub fn upsert(&self) -> DdlOperation {
        DdlOperation::UpsertIndex {
            name: self.name.clone(),
            collection: self.collection.clone(),
            field: self.fields[0].clone(),
            unique: self.unique,
            kind: self.kind.clone(),
            extra_fields: self.fields[1..].to_vec(),
            predicate: self.predicate.clone(),
            analyzer: TextAnalyzer::default(),
        }
    }

    pub fn delete(&self) -> DdlOperation {
        DdlOperation::DeleteIndex {
            name: self.name.clone(),
            collection: self.collection.clone(),
        }
    }
}

pub(super) fn field(name: &str) -> Expr {
    Expr::Operand(Operand::Field(FieldPath::from_fields([name])))
}

pub(super) fn literal(value: impl Into<Value>) -> Expr {
    Expr::Operand(Operand::Literal(value.into()))
}

pub(super) fn binary(op: BinaryOp, left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    }
}

/// Every index the test may create.
fn index_menu(rng: &mut TestRng) -> Vec<IndexSpec> {
    let mut menu = Vec::new();
    for collection in PLAIN {
        let mut name = IndexSpec::new(collection, "nick", IndexKind::Equality, &["nick"]);
        name.unique = rng.one_in(2);
        let mut tag = IndexSpec::new(collection, "tag_pos", IndexKind::Equality, &["tag"]);
        tag.predicate = Some(binary(BinaryOp::Gt, field("n"), literal(0i64)));
        menu.extend([
            name,
            IndexSpec::new(collection, "n", IndexKind::Range, &["n"]),
            IndexSpec::new(collection, "grp_n", IndexKind::Range, &["grp", "n"]),
            tag,
            IndexSpec::new(collection, "body", IndexKind::FullText, &["body"]),
            IndexSpec::new(collection, "ref", IndexKind::Equality, &["ref"]),
        ]);
    }
    menu.push(IndexSpec::new(
        CLASSES,
        "author",
        IndexKind::Equality,
        &[AUTHOR],
    ));
    menu.push(IndexSpec::new(CLASSES, "label", IndexKind::Range, &[LABEL]));
    menu
}

/// The indexes available to the test and those currently created.
#[derive(Debug, Clone)]
pub(super) struct Schema {
    pub menu: Vec<IndexSpec>,
    pub active: Vec<IndexSpec>,
}

impl Schema {
    /// A schema without indexes, for model steps whose unique checks are
    /// deferred (interactive transactions check them at commit).
    pub fn without_indexes() -> Self {
        Self {
            menu: Vec::new(),
            active: Vec::new(),
        }
    }

    pub fn random(rng: &mut TestRng) -> Self {
        let menu = index_menu(rng);
        let active = menu.iter().filter(|_| rng.one_in(2)).cloned().collect();
        Self { menu, active }
    }

    /// Unique fields of `collection`.
    pub fn unique_fields(&self, collection: &str) -> Vec<String> {
        self.active
            .iter()
            .filter(|index| index.unique && index.collection == collection)
            .map(|index| index.fields[0].clone())
            .collect()
    }

    /// Collections, attributes, classes and the initial indexes.
    pub fn setup(&self) -> DdlBatch {
        let mut ddl = DdlBatch::new();
        for collection in PLAIN {
            ddl = ddl.with_op(DdlOperation::UpsertCollection {
                name: collection.into(),
                kind: DdlCollectionKind::Polymorphic,
                integrity_mode: IntegrityMode::Permissive,
            });
        }
        let attribute = |id: &str, kind: TypeKind| DdlOperation::UpsertAttribute {
            attribute: AttributeType {
                id: id.into(),
                name: id.into(),
                ty: Type::new(kind),
                constraints: vec![],
                meta: Meta::default(),
            },
        };
        ddl = ddl
            .with_op(attribute(AUTHOR, TypeKind::Ref(EntityRef::new(PERSON))))
            .with_op(attribute(SUBJECT, TypeKind::Ref(EntityRef::new(DOC))))
            .with_op(attribute(
                LABEL,
                TypeKind::String(StringType {
                    format: None,
                    normalization: None,
                }),
            ));
        for (class, attributes) in [
            (PERSON, vec![LABEL]),
            (DOC, vec![AUTHOR, LABEL]),
            (NOTE, vec![SUBJECT]),
        ] {
            ddl = ddl.with_op(DdlOperation::UpsertClass {
                class: class_type(class, &attributes),
            });
        }
        for index in &self.active {
            ddl = ddl.with_op(index.upsert());
        }
        ddl
    }
}

fn class_type(id: &str, attributes: &[&str]) -> ClassType {
    ClassType {
        id: id.into(),
        name: id.into(),
        inherits: None,
        extends: vec![],
        strict_schema: false,
        creatable_in_ui: None,
        attributes: attributes
            .iter()
            .map(|attribute| {
                (
                    attribute.to_string(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: attribute.to_string(),
                        },
                        required: false,
                        ui_order: None,
                        computed: None,
                        default: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        constraints: vec![],
        meta: Meta::default(),
    }
}
