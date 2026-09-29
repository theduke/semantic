//! Portable operational job metadata. Runtime inputs and outputs never belong here.
use std::collections::BTreeMap;

use crate::schema::*;
use crate::value::{DateTime, FromValue, IntoValue, Object, SemanticType, Value};

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

// The RPC types always encode optional fields, as null, matching the exported Facet types.
// They are keyed by attribute ids; the Facet renames mirror those keys for the SDK export.

#[derive(
    facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, Default, PartialEq, Eq,
)]
#[semantic(namespace = "semantic:jobs:job:progress")]
pub struct JobProgress {
    #[facet(rename = "semantic:jobs:job:progress:completed")]
    pub completed: u64,
    #[semantic(required)]
    #[facet(rename = "semantic:jobs:job:progress:total")]
    pub total: Option<u64>,
    #[semantic(required)]
    #[facet(rename = "semantic:jobs:job:progress:unit")]
    pub unit: Option<String>,
    #[semantic(required)]
    #[facet(rename = "semantic:jobs:job:progress:phase")]
    pub phase: Option<String>,
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
#[semantic(namespace = "semantic:jobs:job:error")]
pub struct JobError {
    #[facet(rename = "semantic:jobs:job:error:code")]
    pub code: String,
    #[facet(rename = "semantic:jobs:job:error:message")]
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
#[semantic(namespace = "semantic:jobs:job_kind")]
pub struct JobKindDescriptor {
    #[facet(rename = "semantic:jobs:job_kind:id")]
    pub id: JobKindId,
    #[semantic(attr = crate::attr::AttrTitle)]
    #[facet(rename = "semantic:title")]
    pub title: String,
    #[semantic(attr = crate::attr::AttrDescription, required)]
    #[facet(rename = "semantic:description")]
    pub description: Option<String>,
}

/// Portable RPC representation, keyed by the job attribute ids.
// Storage uses the separate codec `to_object`/`from_object`, which also carries
// the class and omits unset fields.
#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
#[semantic(namespace = "semantic:jobs:job")]
pub struct JobRecord {
    #[semantic(attr = crate::attr::AttrId)]
    pub id: JobId,
    #[facet(rename = "semantic:jobs:job:kind")]
    pub kind: JobKindId,
    #[facet(rename = "semantic:jobs:job:status")]
    pub status: JobStatus,
    #[facet(rename = "semantic:jobs:job:progress")]
    pub progress: JobProgress,
    #[semantic(required)]
    #[facet(rename = "semantic:jobs:job:error")]
    pub error: Option<JobError>,
    #[facet(rename = "semantic:jobs:job:created_at")]
    pub created_at: DateTime,
    #[semantic(required)]
    #[facet(rename = "semantic:jobs:job:started_at")]
    pub started_at: Option<DateTime>,
    #[facet(rename = "semantic:jobs:job:updated_at")]
    pub updated_at: DateTime,
    #[semantic(required)]
    #[facet(rename = "semantic:jobs:job:finished_at")]
    pub finished_at: Option<DateTime>,
    #[facet(rename = "semantic:jobs:job:snapshot_seq")]
    pub snapshot_seq: u64,
}

#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
#[semantic(namespace = "semantic:jobs:job")]
pub struct JobListCursor {
    #[facet(rename = "semantic:jobs:job:created_at")]
    pub created_at: DateTime,
    #[semantic(attr = crate::attr::AttrId)]
    pub id: JobId,
}

/// Missing fields take their [`Default`] values.
#[derive(facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, PartialEq, Eq)]
#[semantic(namespace = "semantic:jobs")]
pub struct JobListQuery {
    #[semantic(default)]
    #[facet(rename = "semantic:jobs:statuses")]
    pub statuses: Vec<JobStatus>,
    #[facet(rename = "semantic:jobs:kind")]
    pub kind: Option<JobKindId>,
    #[semantic(default)]
    #[facet(rename = "semantic:jobs:oldest_first")]
    pub oldest_first: bool,
    #[facet(rename = "semantic:jobs:cursor")]
    pub cursor: Option<JobListCursor>,
    /// A positive page size.
    #[semantic(default = "JobListQuery::default_limit")]
    #[facet(rename = "semantic:jobs:limit")]
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
#[semantic(namespace = "semantic:jobs")]
pub struct JobListPage {
    #[facet(rename = "semantic:jobs:records")]
    pub records: Vec<JobRecord>,
    #[semantic(required)]
    #[facet(rename = "semantic:jobs:next_cursor")]
    pub next_cursor: Option<JobListCursor>,
}

#[derive(
    facet::Facet, SemanticType, IntoValue, FromValue, Clone, Debug, Default, PartialEq, Eq,
)]
#[semantic(namespace = "semantic:jobs")]
pub struct ClearCompletedResult {
    #[facet(rename = "semantic:jobs:deleted")]
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

    /// Qualified storage codec, including wide counters and native UTC timestamps.
    pub fn to_object(&self) -> Object {
        let mut object = Object::new();
        object.insert(crate::builtin::ATTR_TYPE, CLASS_ID.to_string());
        object.insert(crate::builtin::ATTR_ID, self.id.0.clone());
        let mut put = |name: &str, value: Value| {
            object.insert(format!("{PREFIX}{name}"), value);
        };
        put("kind", self.kind.0.clone().into());
        put("status", self.status.as_str().to_string().into());
        let mut progress = Object::new();
        progress.insert("completed", self.progress.completed);
        if let Some(v) = self.progress.total {
            progress.insert("total", v);
        }
        if let Some(v) = &self.progress.unit {
            progress.insert("unit", v.clone());
        }
        if let Some(v) = &self.progress.phase {
            progress.insert("phase", v.clone());
        }
        put("progress", Value::Object(progress));
        if let Some(error) = &self.error {
            let mut value = Object::new();
            value.insert("code", error.code.clone());
            value.insert("message", error.message.clone());
            put("error", Value::Object(value));
        }
        put("created_at", Value::DateTime(self.created_at));
        put("updated_at", Value::DateTime(self.updated_at));
        if let Some(v) = self.started_at {
            put("started_at", Value::DateTime(v));
        }
        if let Some(v) = self.finished_at {
            put("finished_at", Value::DateTime(v));
        }
        put("snapshot_seq", Value::U64(self.snapshot_seq));
        object
    }

    pub fn from_object(id: JobId, object: &Object) -> Result<Self, String> {
        if object
            .get(crate::builtin::ATTR_TYPE)
            .and_then(Value::as_str)
            != Some(CLASS_ID)
        {
            return Err("invalid job class".into());
        }
        if object.get(crate::builtin::ATTR_ID).and_then(Value::as_str) != Some(id.0.as_str()) {
            return Err("job entity identity mismatch".into());
        }
        if let Some(key) = object.keys().find(|key| {
            key.as_str() != crate::builtin::ATTR_ID
                && key.as_str() != crate::builtin::ATTR_TYPE
                && ![
                    "kind",
                    "status",
                    "progress",
                    "error",
                    "created_at",
                    "started_at",
                    "updated_at",
                    "finished_at",
                    "snapshot_seq",
                ]
                .iter()
                .any(|name| *key == &format!("{PREFIX}{name}"))
        }) {
            return Err(format!("unexpected job metadata field: {key}"));
        }
        let get = |name: &str| object.get(&format!("{PREFIX}{name}"));
        let string = |name: &str| {
            get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("invalid job {name}"))
        };
        let date = |name: &str| -> Result<Option<DateTime>, String> {
            match get(name) {
                None => Ok(None),
                Some(Value::DateTime(v)) => Ok(Some(*v)),
                _ => Err(format!("invalid job {name}")),
            }
        };
        let progress = match get("progress") {
            Some(Value::Object(v)) => v,
            _ => return Err("invalid job progress".into()),
        };
        if progress
            .keys()
            .any(|key| !["completed", "total", "unit", "phase"].contains(&key.as_str()))
        {
            return Err("unexpected job progress field".into());
        }
        let optional_string = |name: &str| -> Result<Option<String>, String> {
            match progress.get(name) {
                None => Ok(None),
                Some(Value::String(v)) => Ok(Some(v.clone())),
                _ => Err(format!("invalid progress {name}")),
            }
        };
        let error = match get("error") {
            None => None,
            Some(Value::Object(v)) => Some(JobError {
                code: v
                    .get("code")
                    .and_then(Value::as_str)
                    .ok_or("invalid error code")?
                    .into(),
                message: v
                    .get("message")
                    .and_then(Value::as_str)
                    .ok_or("invalid error message")?
                    .into(),
            }),
            _ => return Err("invalid job error".into()),
        };
        let record = Self {
            id,
            kind: JobKindId(string("kind")?),
            status: JobStatus::parse(&string("status")?).ok_or("unknown job status")?,
            progress: JobProgress {
                completed: uint(progress.get("completed"))?,
                total: progress.get("total").map(|v| uint(Some(v))).transpose()?,
                unit: optional_string("unit")?,
                phase: optional_string("phase")?,
            },
            error,
            created_at: date("created_at")?.ok_or("missing created_at")?,
            updated_at: date("updated_at")?.ok_or("missing updated_at")?,
            started_at: date("started_at")?,
            finished_at: date("finished_at")?,
            snapshot_seq: uint(get("snapshot_seq"))?,
        };
        record.validate()?;
        Ok(record)
    }
}

fn uint(value: Option<&Value>) -> Result<u64, String> {
    match value {
        Some(Value::U64(v)) => Ok(*v),
        Some(v) => v
            .as_i64()
            .and_then(|v| u64::try_from(v).ok())
            .ok_or("invalid unsigned counter".into()),
        None => Err("missing unsigned counter".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operational_codec_is_lossless_and_has_no_payload_fields() {
        let now = DateTime::now_utc();
        for status in JobStatus::ALL {
            let record = JobRecord {
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
            };
            let object = record.to_object();
            let Value::Object(wire) = record.clone().into_value() else {
                panic!("record object")
            };
            assert_eq!(wire.keys().count(), 10);
            // The RPC form uses the storage keys, minus the class.
            assert!(wire.keys().all(|key| object.contains_key(key)
                || matches!(
                    key.as_str(),
                    "semantic:jobs:job:error"
                        | "semantic:jobs:job:started_at"
                        | "semantic:jobs:job:finished_at"
                )));
            let get = |name: &str| wire.get(&format!("{PREFIX}{name}"));
            assert_eq!(get("kind").and_then(Value::as_str), Some("unknown.kind"));
            assert_eq!(get("created_at"), Some(&Value::DateTime(now)));
            assert_eq!(get("snapshot_seq"), Some(&Value::U64(1)));
            let Some(Value::Object(progress)) = get("progress") else {
                panic!("progress object")
            };
            assert_eq!(
                progress.get("semantic:jobs:job:progress:completed"),
                Some(&Value::U64(u64::MAX))
            );
            assert_eq!(
                progress.get("semantic:jobs:job:progress:unit"),
                Some(&Value::Null)
            );
            assert_eq!(
                JobRecord::from_value(Value::Object(wire.clone())).unwrap(),
                record
            );
            let tagged = crate::value::serde::typed::TypedValue(Value::Object(wire.clone()));
            let json = serde_json::to_string(&tagged).unwrap();
            assert!(json.contains("\"u64\":18446744073709551615"));
            assert!(json.contains("\"date_time\":"));
            assert_eq!(
                serde_json::from_str::<crate::value::serde::typed::TypedValue>(&json).unwrap(),
                tagged
            );
            if status == JobStatus::Queued {
                assert_eq!(get("error"), Some(&Value::Null));
                assert_eq!(get("started_at"), Some(&Value::Null));
                assert_eq!(get("finished_at"), Some(&Value::Null));
            }
            assert_eq!(
                record,
                JobRecord::from_object(record.id.clone(), &object).unwrap()
            );
            assert!(object.keys().all(|k| {
                k == "id"
                    || k == "type"
                    || [
                        "kind",
                        "status",
                        "progress",
                        "error",
                        "created_at",
                        "started_at",
                        "updated_at",
                        "finished_at",
                        "snapshot_seq",
                    ]
                    .iter()
                    .any(|name| k == &format!("{PREFIX}{name}"))
            }));
            let page = JobListPage {
                records: vec![record],
                next_cursor: Some(JobListCursor {
                    id: JobId("job".into()),
                    created_at: now,
                }),
            };
            assert_eq!(
                page,
                JobListPage::from_value(page.clone().into_value()).unwrap()
            );
        }
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
        nulls.insert("semantic:jobs:kind", Value::Null);
        nulls.insert("cursor", Value::Null);
        assert_eq!(
            JobListQuery::from_value(Value::Object(nulls)).unwrap(),
            query
        );
        let TypeKind::Record(record) = JobListQuery::semantic_type().kind else {
            panic!("object query")
        };
        assert!(record.fields.values().all(|field| !field.required));
        assert_eq!(
            record.fields["semantic:jobs:limit"].default,
            Some(Value::U32(50))
        );
        let TypeKind::Record(record) = JobRecord::semantic_type().kind else {
            panic!("object record")
        };
        assert!(record.fields.values().all(|field| field.required));
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
