//! Portable import proposals. Content and requests are never job history payloads.
use crate::{Object, Value, schema::*};
use std::collections::BTreeMap;

pub const PACKAGE_NAME: &str = "semantic.import";
pub const SOURCE_NAMESPACE: &str = "semantic:import:source_namespace";
pub const SOURCE_IDENTITY: &str = "semantic:import:source_identity";
pub const SOURCE_KEY: &str = "semantic:import:source_key";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportError {
    pub code: String,
    pub message: String,
}
impl ImportError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ImportError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRequest {
    pub url: String,
    pub options: Object,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub namespace: String,
    pub source: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedRequest {
    pub identity: SourceIdentity,
    pub representation: String,
    pub options: Object,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Fetch,
    ImportSource,
    ImportFetched,
}
impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::ImportSource => "import_source",
            Self::ImportFetched => "import_fetched",
        }
    }
    pub fn parse(value: &str) -> Result<Self, ImportError> {
        match value {
            "fetch" => Ok(Self::Fetch),
            "import_source" => Ok(Self::ImportSource),
            "import_fetched" => Ok(Self::ImportFetched),
            _ => Err(ImportError::new("invalid_input", "unknown operation")),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDescriptor {
    pub namespace: String,
    pub title: String,
    pub operations: Vec<Operation>,
    pub input_kinds: Vec<String>,
    pub url_schemes: Vec<String>,
    pub default_priority: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeResult {
    Supported,
    Unsupported(String),
    Unavailable(String),
}
impl SourceDescriptor {
    pub fn to_value(&self) -> Value {
        let mut o = Object::new();
        o.insert("namespace", self.namespace.clone());
        o.insert("title", self.title.clone());
        o.insert("default_priority", Value::I32(self.default_priority));
        o.insert(
            "operations",
            Value::List(
                self.operations
                    .iter()
                    .map(|op| Value::String(op.as_str().into()))
                    .collect(),
            ),
        );
        o.insert(
            "input_kinds",
            Value::List(
                self.input_kinds
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
        o.insert(
            "url_schemes",
            Value::List(
                self.url_schemes
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
        Value::Object(o)
    }
    pub fn from_value(value: Value) -> Result<Self, ImportError> {
        fn strings(o: &mut Object, key: &str) -> Result<Vec<String>, ImportError> {
            match o.remove(key) {
                Some(Value::List(values)) => values
                    .into_iter()
                    .map(|v| match v {
                        Value::String(s) => Ok(s),
                        _ => Err(ImportError::new("invalid_input", "expected string list")),
                    })
                    .collect(),
                _ => Err(ImportError::new("invalid_input", key)),
            }
        }
        let mut o = object(value)?;
        Ok(Self {
            namespace: take_string(&mut o, "namespace")?,
            title: take_string(&mut o, "title")?,
            operations: strings(&mut o, "operations")?
                .iter()
                .map(|s| Operation::parse(s))
                .collect::<Result<Vec<_>, ImportError>>()?,
            input_kinds: strings(&mut o, "input_kinds")?,
            url_schemes: strings(&mut o, "url_schemes")?,
            default_priority: match o.remove("default_priority") {
                Some(Value::I32(n)) => n,
                _ => return Err(ImportError::new("invalid_input", "expected i32 priority")),
            },
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityProposal {
    pub key: String,
    pub class: String,
    pub attributes: Object,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileMetadata {
    pub key: String,
    pub filename: Option<String>,
    pub mime_type: String,
    pub expected_size: Option<u64>,
    pub attributes: Object,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentEvent {
    Entity(EntityProposal),
    FileStart(FileMetadata),
    FileBytes { bytes: bytes::Bytes },
    FileEnd,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContentSummary {
    pub items: u64,
    pub bytes: u64,
}

fn object(value: Value) -> Result<Object, ImportError> {
    match value {
        Value::Object(o) => Ok(o),
        _ => Err(ImportError::new("invalid_input", "expected object")),
    }
}
pub fn take_string(o: &mut Object, field: &str) -> Result<String, ImportError> {
    match o.remove(field) {
        Some(Value::String(v)) => Ok(v),
        _ => Err(ImportError::new(
            "invalid_input",
            format!("expected string {field}"),
        )),
    }
}
fn attrs(o: &mut Object) -> Result<Object, ImportError> {
    object(
        o.remove("attributes")
            .unwrap_or(Value::Object(Object::new())),
    )
}
fn opt_string(o: &mut Object, field: &str) -> Result<Option<String>, ImportError> {
    match o.remove(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        _ => Err(ImportError::new("invalid_input", field)),
    }
}
fn uint(o: &mut Object, field: &str) -> Result<u64, ImportError> {
    match o.remove(field) {
        Some(Value::U64(n)) => Ok(n),
        _ => Err(ImportError::new(
            "invalid_input",
            format!("expected u64 {field}"),
        )),
    }
}
impl SourceRequest {
    pub fn to_value(&self) -> Value {
        let mut o = Object::new();
        o.insert("url", self.url.clone());
        o.insert("options", Value::Object(self.options.clone()));
        Value::Object(o)
    }
    pub fn from_value(value: Value) -> Result<Self, ImportError> {
        let mut o = object(value)?;
        Ok(Self {
            url: take_string(&mut o, "url")?,
            options: object(o.remove("options").unwrap_or(Value::Object(Object::new())))?,
        })
    }
}
impl FetchedRequest {
    pub fn to_value(&self) -> Value {
        let mut o = Object::new();
        o.insert("namespace", self.identity.namespace.clone());
        o.insert("source", self.identity.source.clone());
        o.insert("representation", self.representation.clone());
        o.insert("options", Value::Object(self.options.clone()));
        Value::Object(o)
    }
    pub fn from_value(value: Value) -> Result<Self, ImportError> {
        let mut o = object(value)?;
        Ok(Self {
            identity: SourceIdentity {
                namespace: take_string(&mut o, "namespace")?,
                source: take_string(&mut o, "source")?,
            },
            representation: take_string(&mut o, "representation")?,
            options: object(o.remove("options").unwrap_or(Value::Object(Object::new())))?,
        })
    }
}
impl ContentSummary {
    pub fn to_value(&self) -> Value {
        let mut o = Object::new();
        o.insert("items", Value::U64(self.items));
        o.insert("bytes", Value::U64(self.bytes));
        Value::Object(o)
    }
    pub fn from_value(value: Value) -> Result<Self, ImportError> {
        let mut o = object(value)?;
        Ok(Self {
            items: uint(&mut o, "items")?,
            bytes: uint(&mut o, "bytes")?,
        })
    }
}
impl ContentEvent {
    pub fn to_value(&self) -> Value {
        let mut o = Object::new();
        match self {
            Self::Entity(e) => {
                o.insert("kind", String::from("entity"));
                o.insert("key", e.key.clone());
                o.insert("class", e.class.clone());
                o.insert("attributes", Value::Object(e.attributes.clone()));
            }
            Self::FileStart(f) => {
                o.insert("kind", String::from("file_start"));
                o.insert("key", f.key.clone());
                o.insert("mime_type", f.mime_type.clone());
                o.insert(
                    "filename",
                    f.filename.clone().map(Value::String).unwrap_or(Value::Null),
                );
                o.insert(
                    "expected_size",
                    f.expected_size.map(Value::U64).unwrap_or(Value::Null),
                );
                o.insert("attributes", Value::Object(f.attributes.clone()));
            }
            Self::FileBytes { bytes } => {
                o.insert("kind", String::from("file_bytes"));
                o.insert("bytes", Value::Bytes(bytes.clone()));
            }
            Self::FileEnd => {
                o.insert("kind", String::from("file_end"));
            }
        }
        Value::Object(o)
    }
    pub fn from_value(value: Value) -> Result<Self, ImportError> {
        let mut o = object(value)?;
        Ok(match take_string(&mut o, "kind")?.as_str() {
            "entity" => Self::Entity(EntityProposal {
                key: take_string(&mut o, "key")?,
                class: take_string(&mut o, "class")?,
                attributes: attrs(&mut o)?,
            }),
            "file_start" => {
                let expected_size = match o.remove("expected_size") {
                    None | Some(Value::Null) => None,
                    Some(Value::U64(n)) => Some(n),
                    _ => return Err(ImportError::new("invalid_input", "expected_size")),
                };
                Self::FileStart(FileMetadata {
                    key: take_string(&mut o, "key")?,
                    filename: opt_string(&mut o, "filename")?,
                    mime_type: take_string(&mut o, "mime_type")?,
                    expected_size,
                    attributes: attrs(&mut o)?,
                })
            }
            "file_bytes" => Self::FileBytes {
                bytes: match o.remove("bytes") {
                    Some(Value::Bytes(b)) => b,
                    _ => return Err(ImportError::new("invalid_input", "expected bytes")),
                },
            },
            "file_end" => Self::FileEnd,
            _ => return Err(ImportError::new("invalid_input", "unknown content kind")),
        })
    }
}

/// The single declaration consumed by catalog resolution and provider adapters.
pub fn package() -> Package {
    fn string() -> Type {
        Type::new(TypeKind::String(StringType {
            format: None,
            normalization: None,
        }))
    }
    fn uint() -> Type {
        Type::new(TypeKind::Number(NumberType::UInt(UIntWidth::U64)))
    }
    fn record(fields: Vec<(&str, Type, bool)>, open: bool) -> Type {
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
            open,
            additional: None,
            required_order: None,
        }))
    }
    fn optional(t: Type) -> Type {
        Type::new(TypeKind::Optional(OptionalType { inner: Box::new(t) }))
    }
    let attributes = record(vec![], true);
    let request = record(
        vec![
            ("url", string(), true),
            ("options", attributes.clone(), true),
        ],
        false,
    );
    let fetched = record(
        vec![
            ("namespace", string(), true),
            ("source", string(), true),
            ("representation", string(), true),
            ("options", attributes.clone(), true),
        ],
        false,
    );
    let tag = |name: &str| {
        Type::new(TypeKind::Enum(EnumType {
            repr: EnumRepr::String,
            variants: vec![EnumVariant {
                name: name.into(),
                value: None,
                symbol: Some(name.into()),
                meta: Meta::default(),
            }],
        }))
    };
    let event = Type::new(TypeKind::Union(UnionType {
        variants: vec![
            record(
                vec![
                    ("kind", tag("entity"), true),
                    ("key", string(), true),
                    ("class", string(), true),
                    ("attributes", attributes.clone(), true),
                ],
                false,
            ),
            record(
                vec![
                    ("kind", tag("file_start"), true),
                    ("key", string(), true),
                    ("attributes", attributes.clone(), true),
                    ("filename", optional(string()), true),
                    ("mime_type", string(), true),
                    ("expected_size", optional(uint()), true),
                ],
                false,
            ),
            record(
                vec![
                    ("kind", tag("file_bytes"), true),
                    (
                        "bytes",
                        Type::new(TypeKind::Bytes(BytesType { encoding: None })),
                        true,
                    ),
                ],
                false,
            ),
            record(vec![("kind", tag("file_end"), true)], false),
        ],
    }));
    let summary = record(
        vec![("items", uint(), true), ("bytes", uint(), true)],
        false,
    );
    let content = Type::new(TypeKind::Stream(StreamType {
        element: Box::new(event),
        end: Some(Box::new(summary)),
    }));
    let error = record(
        vec![("code", string(), true), ("message", string(), true)],
        false,
    );
    let method = |name: &str, params: Vec<(&str, Type)>, result: Type| InterfaceMethod {
        name: name.into(),
        signature: FunctionType {
            params: params
                .into_iter()
                .map(|(n, ty)| FunctionParam {
                    name: Some(n.into()),
                    ty,
                })
                .collect(),
            results: vec![result],
            throws: Some(Box::new(error.clone())),
            async_fn: true,
        },
    };
    let list = |t: Type| Type::new(TypeKind::List(ListType { items: Box::new(t) }));
    let descriptor = record(
        vec![
            ("namespace", string(), true),
            ("title", string(), true),
            ("operations", list(string()), true),
            ("input_kinds", list(string()), true),
            ("url_schemes", list(string()), true),
            (
                "default_priority",
                Type::new(TypeKind::Number(NumberType::Int(IntWidth::I32))),
                true,
            ),
        ],
        false,
    );
    let probe = record(
        vec![
            ("status", string(), true),
            ("reason", optional(string()), true),
        ],
        false,
    );
    let host_request = record(
        vec![
            ("scope_id", optional(string()), false),
            ("url", string(), true),
            ("options", attributes.clone(), false),
            ("plugin_id", optional(string()), false),
            ("export", optional(string()), false),
            ("generation", optional(uint()), false),
            ("operation", string(), false),
        ],
        false,
    );
    let host_fetched = record(
        vec![
            ("scope_id", optional(string()), false),
            ("namespace", string(), true),
            ("source", string(), true),
            ("representation", string(), true),
            ("options", attributes.clone(), false),
            ("plugin_id", string(), true),
            ("export", string(), true),
            ("generation", optional(uint()), false),
        ],
        false,
    );
    let job_id = record(vec![("id", string(), true)], false);
    let application = InterfaceType {
        methods: vec![
            method(
                "list_candidates",
                vec![("request", host_request.clone())],
                list(record(
                    vec![
                        ("plugin_id", string(), true),
                        ("export", string(), true),
                        ("source_export", string(), true),
                        ("generation", uint(), true),
                        (
                            "priority",
                            Type::new(TypeKind::Number(NumberType::Int(IntWidth::I32))),
                            true,
                        ),
                        ("descriptor", descriptor.clone(), true),
                        ("status", string(), true),
                        ("reason", optional(string()), true),
                    ],
                    false,
                )),
            ),
            method(
                "fetch_source",
                vec![("request", host_request.clone())],
                content.clone(),
            ),
            method(
                "start_import_source",
                vec![("request", host_request)],
                job_id.clone(),
            ),
            method(
                "start_import_fetched",
                vec![("request", host_fetched), ("content", content.clone())],
                job_id,
            ),
        ],
    };
    let interfaces = BTreeMap::from([
        ("Application".into(), application),
        (
            "Source".into(),
            InterfaceType {
                methods: vec![
                    method("describe", vec![], descriptor),
                    method(
                        "probe",
                        vec![("request", request.clone()), ("operation", string())],
                        probe,
                    ),
                ],
            },
        ),
        (
            "Fetcher".into(),
            InterfaceType {
                methods: vec![method(
                    "fetch",
                    vec![("request", request.clone())],
                    content.clone(),
                )],
            },
        ),
        (
            "Importer".into(),
            InterfaceType {
                methods: vec![
                    method("import_source", vec![("request", request)], content.clone()),
                    method(
                        "import_fetched",
                        vec![("request", fetched), ("content", content.clone())],
                        content,
                    ),
                ],
            },
        ),
    ]);
    let attributes: BTreeMap<String, AttributeType> = [
        (SOURCE_NAMESPACE, "Source namespace"),
        (SOURCE_IDENTITY, "Source identity"),
        (SOURCE_KEY, "Source key"),
    ]
    .into_iter()
    .map(|(id, name)| {
        (
            id.into(),
            AttributeType {
                id: id.into(),
                name: name.into(),
                ty: string(),
                constraints: vec![],
                meta: Meta::default(),
            },
        )
    })
    .collect();
    let operations = attributes
        .values()
        .cloned()
        .map(|attribute| {
            MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute })
        })
        .collect();
    Package {
        name: PACKAGE_NAME.into(),
        root: Module {
            name: "v1".into(),
            constants: BTreeMap::new(),
            types: BTreeMap::new(),
            attributes,
            classes: BTreeMap::new(),
            interfaces,
            contracts: BTreeMap::new(),
            meta: Meta::default(),
        },
        modules: BTreeMap::new(),
        migrations: vec![Migration {
            module: "v1".into(),
            name: "001_source_attributes".into(),
            description: Some("Ordinary source provenance attributes".into()),
            operations,
            meta: Meta::default(),
        }],
        version: Some(SchemaVersion {
            major: 1,
            minor: 0,
            patch: 0,
            pre: None,
            build: None,
        }),
        meta: Meta::default(),
    }
}
