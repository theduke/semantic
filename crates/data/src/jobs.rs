//! Portable operational job metadata. Runtime inputs and outputs never belong here.
use std::collections::BTreeMap;

use crate::schema::*;
use crate::value::{DateTime, Object, Value};

pub const PACKAGE_NAME: &str = "semantic.jobs";
pub const COLLECTION: &str = "semantic_jobs";
pub const CLASS_ID: &str = "semantic:jobs:job";
pub const PREFIX: &str = "semantic:jobs:job:";

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[facet(transparent)]
pub struct JobId(pub String);

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[facet(transparent)]
pub struct JobKindId(pub String);

#[derive(facet::Facet, Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
#[facet(rename_all = "snake_case")]
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

#[derive(facet::Facet, Clone, Debug, Default, PartialEq, Eq)]
pub struct JobProgress {
    pub completed: u64,
    pub total: Option<u64>,
    pub unit: Option<String>,
    pub phase: Option<String>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
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

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct JobKindDescriptor {
    pub id: JobKindId,
    pub title: String,
    pub description: Option<String>,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct JobRecord {
    pub id: JobId,
    pub kind: JobKindId,
    pub status: JobStatus,
    pub progress: JobProgress,
    pub error: Option<JobError>,
    pub created_at: DateTime,
    pub started_at: Option<DateTime>,
    pub updated_at: DateTime,
    pub finished_at: Option<DateTime>,
    pub snapshot_seq: u64,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct JobListCursor {
    pub created_at: DateTime,
    pub id: JobId,
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct JobListQuery {
    pub statuses: Vec<JobStatus>,
    pub kind: Option<JobKindId>,
    pub oldest_first: bool,
    pub cursor: Option<JobListCursor>,
    pub limit: u32,
}
impl Default for JobListQuery {
    fn default() -> Self {
        Self {
            statuses: Vec::new(),
            kind: None,
            oldest_first: false,
            cursor: None,
            limit: 50,
        }
    }
}

impl JobListQuery {
    pub fn to_object(&self) -> Object {
        let mut object = Object::new();
        object.insert(
            "statuses",
            Value::List(
                self.statuses
                    .iter()
                    .map(|s| Value::String(s.as_str().into()))
                    .collect(),
            ),
        );
        object.insert(
            "kind",
            self.kind
                .as_ref()
                .map(|v| Value::String(v.0.clone()))
                .unwrap_or(Value::Null),
        );
        object.insert("oldest_first", self.oldest_first);
        object.insert("limit", self.limit as u64);
        object.insert(
            "cursor",
            self.cursor
                .as_ref()
                .map(|v| Value::Object(v.to_object()))
                .unwrap_or(Value::Null),
        );
        object
    }
    pub fn from_object(object: &Object) -> Result<Self, String> {
        let statuses = match object.get("statuses") {
            None => Vec::new(),
            Some(Value::List(values)) => values
                .iter()
                .map(|v| {
                    v.as_str()
                        .and_then(JobStatus::parse)
                        .ok_or_else(|| "invalid job status filter".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
            _ => return Err("invalid statuses".into()),
        };
        let kind = match object.get("kind") {
            None | Some(Value::Null) => None,
            Some(Value::String(v)) => Some(JobKindId(v.clone())),
            _ => return Err("invalid kind".into()),
        };
        let oldest_first = match object.get("oldest_first") {
            None => false,
            Some(Value::Bool(v)) => *v,
            _ => return Err("invalid oldest_first".into()),
        };
        let limit = match object.get("limit") {
            None => 50,
            Some(v) => u32::try_from(uint(Some(v))?).map_err(|_| "invalid limit")?,
        };
        if limit == 0 {
            return Err("limit must be positive".into());
        }
        let cursor = match object.get("cursor") {
            None | Some(Value::Null) => None,
            Some(Value::Object(v)) => Some(JobListCursor::from_object(v)?),
            _ => return Err("invalid cursor".into()),
        };
        Ok(Self {
            statuses,
            kind,
            oldest_first,
            limit,
            cursor,
        })
    }
}
impl JobListCursor {
    fn to_object(&self) -> Object {
        let mut object = Object::new();
        object.insert("id", self.id.0.clone());
        object.insert("created_at", Value::DateTime(self.created_at));
        object
    }
    fn from_object(object: &Object) -> Result<Self, String> {
        Ok(Self {
            id: JobId(
                object
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("invalid cursor id")?
                    .into(),
            ),
            created_at: match object.get("created_at") {
                Some(Value::DateTime(v)) => *v,
                _ => return Err("invalid cursor created_at".into()),
            },
        })
    }
}

#[derive(facet::Facet, Clone, Debug, PartialEq, Eq)]
pub struct JobListPage {
    pub records: Vec<JobRecord>,
    pub next_cursor: Option<JobListCursor>,
}

impl JobListPage {
    pub fn to_value(&self) -> Value {
        let mut object = Object::new();
        object.insert(
            "records",
            Value::List(
                self.records
                    .iter()
                    .map(|r| Value::Object(r.to_rpc_object()))
                    .collect(),
            ),
        );
        object.insert(
            "next_cursor",
            self.next_cursor
                .as_ref()
                .map(|c| Value::Object(c.to_object()))
                .unwrap_or(Value::Null),
        );
        Value::Object(object)
    }
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let Value::Object(object) = value else {
            return Err("invalid jobs page".into());
        };
        let Some(Value::List(rows)) = object.get("records") else {
            return Err("missing jobs records".into());
        };
        let records = rows
            .iter()
            .map(|v| {
                let Value::Object(row) = v else {
                    return Err("invalid job row".into());
                };
                JobRecord::from_rpc_object(row)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let next_cursor = match object.get("next_cursor") {
            None | Some(Value::Null) => None,
            Some(Value::Object(v)) => Some(JobListCursor::from_object(v)?),
            _ => return Err("invalid next cursor".into()),
        };
        Ok(Self {
            records,
            next_cursor,
        })
    }
}

#[derive(facet::Facet, Clone, Debug, Default, PartialEq, Eq)]
pub struct ClearCompletedResult {
    pub deleted: u64,
}

impl JobRecord {
    /// Portable RPC representation; storage attribute names never cross this boundary.
    /// Optional fields are present with null values, matching the exported Facet types.
    pub fn to_rpc_object(&self) -> Object {
        let storage = self.to_object();
        let mut object = Object::new();
        object.insert("id", self.id.0.clone());
        for name in [
            "kind",
            "status",
            "progress",
            "error",
            "created_at",
            "started_at",
            "updated_at",
            "finished_at",
            "snapshot_seq",
        ] {
            let mut value = storage
                .get(&format!("{PREFIX}{name}"))
                .cloned()
                .unwrap_or(Value::Null);
            if let Value::Object(progress) = &mut value {
                if name == "progress" {
                    for field in ["total", "unit", "phase"] {
                        if !progress.contains_key(field) {
                            progress.insert(field, Value::Null);
                        }
                    }
                }
            }
            object.insert(name, value);
        }
        object
    }

    pub fn from_rpc_object(object: &Object) -> Result<Self, String> {
        let id = JobId(
            object
                .get("id")
                .and_then(Value::as_str)
                .ok_or("missing job id")?
                .into(),
        );
        let mut storage = Object::new();
        storage.insert(crate::builtin::ATTR_ID, id.0.clone());
        storage.insert(crate::builtin::ATTR_TYPE, CLASS_ID.to_string());
        for (name, value) in object.iter() {
            if name == "id" {
                continue;
            }
            if matches!(name.as_str(), "error" | "started_at" | "finished_at")
                && matches!(value, Value::Null)
            {
                continue;
            }
            let mut value = value.clone();
            if name == "progress" {
                if let Value::Object(progress) = &mut value {
                    for field in ["total", "unit", "phase"] {
                        if matches!(progress.get(field), Some(Value::Null)) {
                            progress.remove(field);
                        }
                    }
                }
            }
            storage.insert(format!("{PREFIX}{name}"), value);
        }
        Self::from_object(id, &storage)
    }

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
            let wire = record.to_rpc_object();
            assert_eq!(wire.keys().count(), 10);
            assert!(wire.keys().all(|key| !key.contains(':')));
            assert_eq!(
                wire.get("kind").and_then(Value::as_str),
                Some("unknown.kind")
            );
            assert_eq!(wire.get("created_at"), Some(&Value::DateTime(now)));
            assert_eq!(wire.get("snapshot_seq"), Some(&Value::U64(1)));
            let Some(Value::Object(progress)) = wire.get("progress") else {
                panic!("progress object")
            };
            assert_eq!(progress.get("completed"), Some(&Value::U64(u64::MAX)));
            assert_eq!(progress.get("unit"), Some(&Value::Null));
            assert_eq!(JobRecord::from_rpc_object(&wire).unwrap(), record);
            let tagged = crate::value::serde::typed::TypedValue(Value::Object(wire.clone()));
            let json = serde_json::to_string(&tagged).unwrap();
            assert!(json.contains("\"u64\":18446744073709551615"));
            assert!(json.contains("\"date_time\":"));
            assert!(!json.contains(PREFIX));
            assert_eq!(
                serde_json::from_str::<crate::value::serde::typed::TypedValue>(&json).unwrap(),
                tagged
            );
            if status == JobStatus::Queued {
                assert_eq!(wire.get("error"), Some(&Value::Null));
                assert_eq!(wire.get("started_at"), Some(&Value::Null));
                assert_eq!(wire.get("finished_at"), Some(&Value::Null));
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
            assert_eq!(page, JobListPage::from_value(&page.to_value()).unwrap());
        }
    }

    #[test]
    fn rpc_query_nulls_and_signatures_match_the_contract() {
        let query = JobListQuery::default();
        assert_eq!(
            JobListQuery::from_object(&query.to_object()).unwrap(),
            query
        );
        assert_eq!(query.to_object().get("kind"), Some(&Value::Null));
        for command in ["list", "get", "cancel", "clear_completed", "kinds"] {
            let signature = command_signature(&format!("semantic.jobs.{command}")).unwrap();
            assert_eq!(signature.params.len(), 1);
            assert_eq!(signature.results.len(), 1);
            let TypeKind::Record(payload) = &signature.params[0].ty.kind else {
                panic!("object payload")
            };
            assert!(!payload.fields["scope_id"].required);
            if matches!(command, "get" | "cancel") {
                assert!(payload.fields["id"].required);
            }
        }
    }
}

/// Portable command schemas, independent of qualified database attributes.
pub fn command_signature(name: &str) -> Option<FunctionType> {
    fn string() -> Type {
        Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        }))
    }
    fn optional(ty: Type) -> Type {
        Type::new(TypeKind::Optional(OptionalType {
            inner: Box::new(ty),
        }))
    }
    fn list(ty: Type) -> Type {
        Type::new(TypeKind::List(ListType {
            items: Box::new(ty),
        }))
    }
    fn record(fields: Vec<(&str, Type, bool)>) -> Type {
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
    }
    let uint = || Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64)));
    let date = || Type::new(TypeKind::Temporal(TemporalType::DateTime));
    let status = Type::new(TypeKind::Enum(EnumType {
        repr: EnumRepr::String,
        variants: JobStatus::ALL
            .into_iter()
            .map(|status| EnumVariant {
                name: status.as_str().into(),
                symbol: Some(status.as_str().into()),
                value: None,
                meta: Meta::default(),
            })
            .collect(),
    }));
    let cursor = record(vec![("id", string(), true), ("created_at", date(), true)]);
    let job = record(vec![
        ("id", string(), true),
        ("kind", string(), true),
        ("status", status.clone(), true),
        (
            "progress",
            record(vec![
                ("completed", uint(), true),
                ("total", optional(uint()), true),
                ("unit", optional(string()), true),
                ("phase", optional(string()), true),
            ]),
            true,
        ),
        (
            "error",
            optional(record(vec![
                ("code", string(), true),
                ("message", string(), true),
            ])),
            true,
        ),
        ("created_at", date(), true),
        ("started_at", optional(date()), true),
        ("updated_at", date(), true),
        ("finished_at", optional(date()), true),
        ("snapshot_seq", uint(), true),
    ]);
    let mut params = vec![("scope_id", optional(string()), false)];
    let result = match name {
        "semantic.jobs.list" => {
            params.extend([
                ("statuses", list(status), false),
                ("kind", optional(string()), false),
                ("oldest_first", Type::new_bool(), false),
                ("cursor", optional(cursor.clone()), false),
                (
                    "limit",
                    Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U32))),
                    false,
                ),
            ]);
            record(vec![
                ("records", list(job), true),
                ("next_cursor", optional(cursor), true),
            ])
        }
        "semantic.jobs.get" | "semantic.jobs.cancel" => {
            params.push(("id", string(), true));
            if name == "semantic.jobs.get" {
                optional(job)
            } else {
                job
            }
        }
        "semantic.jobs.clear_completed" => record(vec![("deleted", uint(), true)]),
        "semantic.jobs.kinds" => list(record(vec![
            ("id", string(), true),
            ("title", string(), true),
            ("description", optional(string()), true),
        ])),
        _ => return None,
    };
    Some(FunctionType {
        params: vec![FunctionParam {
            name: Some("payload".into()),
            ty: record(params),
        }],
        results: vec![result],
        throws: None,
        async_fn: true,
    })
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
