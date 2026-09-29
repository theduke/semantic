//! Portable operational job metadata. Runtime inputs and outputs never belong here.
use std::collections::BTreeMap;

use crate::schema::*;
use crate::value::{Class, DateTime, FromValue, IntoValue, SemanticType};

pub const PACKAGE_NAME: &str = "semantic.jobs";
pub const COLLECTION: &str = "semantic_jobs";
pub const CLASS_ID: &str = "semantic:jobs:job";
pub const PREFIX: &str = "semantic:jobs:job:";

#[derive(
    facet::Facet,
    SemanticType,
    IntoValue,
    FromValue,
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
#[facet(transparent)]
pub struct JobId(pub String);

#[derive(
    facet::Facet,
    SemanticType,
    IntoValue,
    FromValue,
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
#[facet(transparent)]
pub struct JobKindId(pub String);

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
#[semantic(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
}

impl JobStatus {
    pub const ALL: [Self; 7] = [
        Self::Queued,
        Self::Running,
        Self::Cancelling,
        Self::Succeeded,
        Self::Failed,
        Self::Cancelled,
        Self::Interrupted,
    ];
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == value)
    }
}

// Jobs are entities of the class `semantic:jobs:job`, with one derived codec for
// storage and RPC. Their progress and error are the plain records declared by the
// `001_init` migration. Unset optional job fields are omitted, since the declared
// fields are optional but not nullable; the Facet attributes mirror that wire shape
// for the SDK export.

#[derive(
    facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, Default, PartialEq, Eq,
)]
pub struct JobProgress {
    pub completed: u64,
    #[facet(skip_serializing_if = Option::is_none)]
    pub total: Option<u64>,
    #[facet(skip_serializing_if = Option::is_none)]
    pub unit: Option<String>,
    #[facet(skip_serializing_if = Option::is_none)]
    pub phase: Option<String>,
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct JobError {
    pub code: String,
    pub message: String,
}

impl JobError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for JobError {}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct JobKindDescriptor {
    pub id: JobKindId,
    pub title: String,
    #[semantic(required)]
    pub description: Option<String>,
}

/// A job entity, as stored and as returned over RPC.
#[derive(facet::Facet, Class, Clone, Debug, PartialEq, Eq)]
#[semantic(id = "semantic:jobs:job")]
pub struct JobRecord {
    #[semantic(attr = crate::attr::AttrId)]
    pub id: JobId,
    #[facet(rename = "semantic:jobs:job:kind")]
    pub kind: JobKindId,
    #[facet(rename = "semantic:jobs:job:status")]
    pub status: JobStatus,
    #[facet(rename = "semantic:jobs:job:progress")]
    pub progress: JobProgress,
    #[facet(
        rename = "semantic:jobs:job:error",
        skip_serializing_if = Option::is_none
    )]
    pub error: Option<JobError>,
    #[facet(rename = "semantic:jobs:job:created_at")]
    pub created_at: DateTime,
    #[facet(
        rename = "semantic:jobs:job:started_at",
        skip_serializing_if = Option::is_none
    )]
    pub started_at: Option<DateTime>,
    #[facet(rename = "semantic:jobs:job:updated_at")]
    pub updated_at: DateTime,
    #[facet(
        rename = "semantic:jobs:job:finished_at",
        skip_serializing_if = Option::is_none
    )]
    pub finished_at: Option<DateTime>,
    #[facet(rename = "semantic:jobs:job:snapshot_seq")]
    pub snapshot_seq: u64,
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct JobListCursor {
    pub created_at: DateTime,
    pub id: JobId,
}

/// Missing fields take their [`Default`] values.
#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct JobListQuery {
    #[semantic(default)]
    pub statuses: Vec<JobStatus>,
    pub kind: Option<JobKindId>,
    #[semantic(default)]
    pub oldest_first: bool,
    pub cursor: Option<JobListCursor>,
    /// A positive page size.
    #[semantic(default = "JobListQuery::default_limit")]
    pub limit: u32,
}
impl Default for JobListQuery {
    fn default() -> Self {
        Self {
            statuses: Vec::new(),
            kind: None,
            oldest_first: false,
            cursor: None,
            limit: Self::default_limit(),
        }
    }
}

impl JobListQuery {
    fn default_limit() -> u32 {
        50
    }
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
pub struct JobListPage {
    pub records: Vec<JobRecord>,
    #[semantic(required)]
    pub next_cursor: Option<JobListCursor>,
}

#[derive(
    facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, Default, PartialEq, Eq,
)]
pub struct ClearCompletedResult {
    pub deleted: u64,
}

impl JobRecord {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.0.is_empty() || self.kind.0.is_empty() {
            return Err("empty job identity/kind".into());
        }
        if self.snapshot_seq == 0 {
            return Err("zero job snapshot sequence".into());
        }
        if self.status == JobStatus::Queued && self.started_at.is_some() {
            return Err("queued job has started_at".into());
        }
        if self.status.is_terminal() != self.finished_at.is_some() {
            return Err("job terminal status/finished_at mismatch".into());
        }
        if matches!(
            self.status,
            JobStatus::Running | JobStatus::Cancelling | JobStatus::Succeeded | JobStatus::Failed
        ) && self.started_at.is_none()
        {
            return Err("started job missing started_at".into());
        }
        if self.status == JobStatus::Succeeded && self.error.is_some() {
            return Err("successful job has an error".into());
        }
        if matches!(self.status, JobStatus::Failed | JobStatus::Interrupted) && self.error.is_none()
        {
            return Err("failed/interrupted job missing error".into());
        }
        if self
            .progress
            .total
            .is_some_and(|total| total < self.progress.completed)
        {
            return Err("progress exceeds total".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Object, Value};

    fn record(status: JobStatus) -> JobRecord {
        let now = DateTime::now_utc();
        JobRecord {
            id: JobId("job".into()),
            kind: JobKindId("unknown.kind".into()),
            status,
            progress: JobProgress {
                completed: u64::MAX,
                total: Some(u64::MAX),
                ..Default::default()
            },
            error: matches!(status, JobStatus::Failed | JobStatus::Interrupted)
                .then(|| JobError::new("error", "message")),
            created_at: now,
            updated_at: now,
            started_at: (status != JobStatus::Queued).then_some(now),
            finished_at: status.is_terminal().then_some(now),
            snapshot_seq: 1,
        }
    }

    fn object(value: Value) -> Object {
        match value {
            Value::Object(object) => object,
            other => panic!("expected object, found {other:?}"),
        }
    }

    /// The field names of a declared record type.
    fn record_fields(ty: &Type) -> Vec<&str> {
        match &ty.kind {
            TypeKind::Record(record) => record.fields.keys().map(String::as_str).collect(),
            other => panic!("expected record type, found {other:?}"),
        }
    }

    #[test]
    fn codec_matches_the_declared_class_and_is_lossless() {
        let package = package();
        let attributes = &package.root.attributes;
        let class = &package.root.classes[CLASS_ID];
        for status in JobStatus::ALL {
            let record = record(status);
            let wire = object(record.clone().into_value());
            assert_eq!(wire.get("type").and_then(Value::as_str), Some(CLASS_ID));
            assert_eq!(wire.get("id").and_then(Value::as_str), Some("job"));
            for (key, value) in wire.iter() {
                if key == "id" || key == "type" {
                    continue;
                }
                let attribute = attributes
                    .get(key)
                    .unwrap_or_else(|| panic!("undeclared job attribute {key}"));
                assert_ne!(value, &Value::Null, "{key} is not nullable");
                if let Value::Object(fields) = value {
                    let declared = record_fields(&attribute.ty);
                    assert!(fields.keys().all(|name| declared.contains(&name.as_str())));
                    assert!(fields.values().all(|value| value != &Value::Null));
                }
            }
            for attribute in class.attributes.values().filter(|a| a.required) {
                assert!(wire.contains_key(&attribute.attribute.id));
            }
            let get = |name: &str| wire.get(&format!("{PREFIX}{name}"));
            assert_eq!(get("status").and_then(Value::as_str), Some(status.as_str()));
            assert_eq!(get("created_at"), Some(&Value::DateTime(record.created_at)));
            assert_eq!(get("snapshot_seq"), Some(&Value::U64(1)));
            let progress = object(get("progress").cloned().unwrap());
            assert_eq!(progress.get("completed"), Some(&Value::U64(u64::MAX)));
            assert_eq!(progress.get("unit"), None);
            if status == JobStatus::Queued {
                assert_eq!(get("error"), None);
                assert_eq!(get("started_at"), None);
                assert_eq!(get("finished_at"), None);
            }
            assert_eq!(
                JobRecord::from_value(Value::Object(wire.clone())).unwrap(),
                record
            );
            let tagged = crate::value::serde::typed::TypedValue(Value::Object(wire));
            let json = serde_json::to_string(&tagged).unwrap();
            assert!(json.contains("\"u64\":18446744073709551615"));
            assert!(json.contains("\"date_time\":"));
            assert_eq!(
                serde_json::from_str::<crate::value::serde::typed::TypedValue>(&json).unwrap(),
                tagged
            );
            let page = JobListPage {
                records: vec![record.clone()],
                next_cursor: Some(JobListCursor {
                    id: JobId("job".into()),
                    created_at: record.created_at,
                }),
            };
            assert_eq!(
                page,
                JobListPage::from_value(page.clone().into_value()).unwrap()
            );
        }
    }

    #[test]
    fn decoding_rejects_other_classes() {
        assert_eq!(
            <JobRecord as crate::attr::ClassDescriptorConst>::ID,
            CLASS_ID
        );
        let mut wire = object(record(JobStatus::Queued).into_value());
        wire.insert("type", Value::String("semantic:other".into()));
        assert!(JobRecord::from_value(Value::Object(wire)).is_err());
    }

    #[test]
    fn rpc_query_defaults_and_types_match_the_contract() {
        let query = JobListQuery::default();
        assert_eq!(
            JobListQuery::from_value(query.clone().into_value()).unwrap(),
            query
        );
        assert_eq!(
            JobListQuery::from_value(Value::Object(Object::new())).unwrap(),
            query
        );
        let mut nulls = Object::new();
        nulls.insert("kind", Value::Null);
        nulls.insert("cursor", Value::Null);
        assert_eq!(
            JobListQuery::from_value(Value::Object(nulls)).unwrap(),
            query
        );
        let TypeKind::Record(record) = JobListQuery::semantic_type().kind else {
            panic!("object query")
        };
        assert!(record.fields.values().all(|field| !field.required));
        assert_eq!(record.fields["limit"].default, Some(Value::U32(50)));
        let TypeKind::Record(record) = JobRecord::semantic_type().kind else {
            panic!("object record")
        };
        let class = &package().root.classes[CLASS_ID];
        for attribute in class.attributes.values() {
            assert_eq!(
                record.fields[&attribute.attribute.id].required,
                attribute.required
            );
        }
        assert!(matches!(
            record.fields["semantic:jobs:job:status"].ty.kind,
            TypeKind::Enum(EnumType {
                repr: EnumRepr::String,
                ..
            })
        ));
    }
}

pub fn package() -> Package {
    let string = || {
        Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        }))
    };
    let uint = || Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64)));
    let date = || Type::new(TypeKind::Temporal(TemporalType::DateTime));
    let record = |fields: Vec<(&str, Type, bool)>| {
        Type::new(TypeKind::Record(RecordType {
            fields: fields
                .into_iter()
                .map(|(name, ty, required)| {
                    (
                        name.into(),
                        Field {
                            ty,
                            required,
                            readonly: false,
                            writeonly: false,
                            default: None,
                            meta: Meta::default(),
                        },
                    )
                })
                .collect(),
            open: false,
            additional: None,
            required_order: None,
        }))
    };
    let progress = record(vec![
        ("completed", uint(), true),
        ("total", uint(), false),
        ("unit", string(), false),
        ("phase", string(), false),
    ]);
    let error = record(vec![("code", string(), true), ("message", string(), true)]);
    let fields = vec![
        ("kind", string(), true),
        ("status", string(), true),
        ("progress", progress, true),
        ("error", error, false),
        ("created_at", date(), true),
        ("started_at", date(), false),
        ("updated_at", date(), true),
        ("finished_at", date(), false),
        ("snapshot_seq", uint(), true),
    ];
    let attributes: BTreeMap<_, _> = fields
        .iter()
        .map(|(name, ty, _)| {
            let id = format!("{PREFIX}{name}");
            (
                id.clone(),
                AttributeType {
                    id,
                    name: (*name).into(),
                    ty: ty.clone(),
                    constraints: vec![],
                    meta: Meta::default(),
                },
            )
        })
        .collect();
    let class = ClassType {
        id: CLASS_ID.into(),
        name: "Job".into(),
        inherits: None,
        extends: vec![],
        strict_schema: true,
        creatable_in_ui: Some(false),
        attributes: fields
            .iter()
            .map(|(name, _, required)| {
                (
                    (*name).into(),
                    ClassAttribute {
                        attribute: AttributeRef {
                            id: format!("{PREFIX}{name}"),
                        },
                        required: *required,
                        ui_order: None,
                        computed: None,
                        default: None,
                        constraints: vec![],
                        meta: Meta::default(),
                    },
                )
            })
            .collect(),
        constraints: vec![],
        meta: Meta::default(),
    };
    let mut operations: Vec<_> = attributes
        .values()
        .cloned()
        .map(|attribute| {
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute })
        })
        .collect();
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertClass {
            class: class.clone(),
        },
    ));
    operations.push(MigrationOperation::Ddl(
        MigrationDdlOperation::UpsertCollection {
            name: COLLECTION.into(),
            kind: MigrationCollectionKind::Polymorphic,
            integrity_mode: MigrationIntegrityMode::StrictRegisteredSchema,
        },
    ));
    for field in ["status", "created_at", "kind"] {
        operations.push(MigrationOperation::Ddl(
            MigrationDdlOperation::UpsertIndex {
                name: format!("jobs_{field}"),
                collection: COLLECTION.into(),
                field: format!("{PREFIX}{field}"),
                unique: false,
                kind: Default::default(),
                extra_fields: Vec::new(),
                predicate: None,
                analyzer: Default::default(),
            },
        ));
    }
    Package {
        name: PACKAGE_NAME.into(),
        root: Module {
            name: "v1".into(),
            constants: BTreeMap::new(),
            types: BTreeMap::new(),
            attributes,
            classes: BTreeMap::from([(CLASS_ID.into(), class)]),
            interfaces: BTreeMap::new(),
            contracts: BTreeMap::new(),
            meta: Meta::default(),
        },
        modules: BTreeMap::new(),
        migrations: vec![Migration {
            module: "v1".into(),
            name: "001_init".into(),
            description: Some("Operational job history".into()),
            operations,
            meta: Meta::default(),
        }],
        version: None,
        meta: Meta::default(),
    }
}
