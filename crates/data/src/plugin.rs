//! Portable desired plugin activation state. Live instances never enter storage.
use crate::{Object, Value, schema::*};
use std::collections::BTreeMap;

pub const PACKAGE_NAME: &str = "semantic.plugin";
pub const COLLECTION: &str = "semantic_plugins";
pub const CLASS_ID: &str = "semantic:plugin:activation";
pub const DESCRIPTOR_ATTR: &str = "semantic:plugin:activation:descriptor";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginExport {
    pub export: String,
    pub package: String,
    pub module: String,
    pub contract: Option<String>,
    pub interface: String,
    pub package_version: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginProvider {
    Rust {
        key: String,
    },
    Stdio {
        program: String,
        args: Vec<String>,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
    },
    WebSocket {
        url: String,
    },
}
impl PluginProvider {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Rust { .. } => "rust",
            Self::Stdio { .. } => "stdio",
            Self::WebSocket { .. } => "websocket",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginState {
    Disabled,
    Starting,
    Ready,
    Stopping,
    Unavailable,
    Incompatible,
}
impl PluginState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Stopping => "stopping",
            Self::Unavailable => "unavailable",
            Self::Incompatible => "incompatible",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct PluginConfigurationSchema {
    pub ty: Type,
    pub definitions: BTreeMap<String, TypeDef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginActivation {
    pub id: String,
    pub revision: String,
    pub provider: PluginProvider,
    pub enabled: bool,
    pub generation: u64,
    pub configuration: Value,
    /// Authoritative installation schema, using ordinary package types.
    pub configuration_schema: Option<PluginConfigurationSchema>,
    pub priority: Option<i32>,
    pub exports: Vec<PluginExport>,
    /// Fetcher/Importer export names mapped to their describing Source export.
    /// An omitted mapping is inferred only for a single Source export.
    pub source_bindings: BTreeMap<String, String>,
}
impl PluginActivation {
    pub fn source_for_export(&self, export: &str) -> Option<&str> {
        if let Some(source) = self.source_bindings.get(export) {
            return Some(source);
        }
        let mut sources = self.exports.iter().filter(|export| {
            export.package == crate::import::PACKAGE_NAME && export.interface == "Source"
        });
        let source = sources.next()?;
        sources.next().is_none().then_some(source.export.as_str())
    }

    pub fn validate(&self) -> Result<(), String> {
        let mut names = std::collections::BTreeSet::new();
        for export in &self.exports {
            if export.export.is_empty()
                || !names.insert(&export.export)
                || export.package.is_empty()
                || export.module.is_empty()
                || export.interface.is_empty()
                || export.fingerprint.is_empty()
            {
                return Err("invalid or duplicate plugin export".into());
            }
        }
        for (operation, source) in &self.source_bindings {
            let operation_valid = self.exports.iter().any(|export| {
                export.export == *operation
                    && export.package == crate::import::PACKAGE_NAME
                    && matches!(export.interface.as_str(), "Fetcher" | "Importer")
            });
            let source_valid = self.exports.iter().any(|export| {
                export.export == *source
                    && export.package == crate::import::PACKAGE_NAME
                    && export.interface == "Source"
            });
            if !operation_valid || !source_valid {
                return Err(
                    "source binding must connect a Fetcher/Importer export to a Source export"
                        .into(),
                );
            }
        }
        let source_count = self
            .exports
            .iter()
            .filter(|export| {
                export.package == crate::import::PACKAGE_NAME && export.interface == "Source"
            })
            .count();
        if source_count > 1
            && self.exports.iter().any(|export| {
                export.package == crate::import::PACKAGE_NAME
                    && matches!(export.interface.as_str(), "Fetcher" | "Importer")
                    && !self.source_bindings.contains_key(&export.export)
            })
        {
            return Err("multiple Source exports require explicit source bindings for every Fetcher/Importer".into());
        }
        if self.id.is_empty() || self.revision.is_empty() || self.generation == 0 {
            return Err("id, revision and positive generation are required".into());
        }
        match &self.provider {
            PluginProvider::Rust { key } if key.is_empty() => {
                return Err("empty Rust registration key".into());
            }
            PluginProvider::Stdio { program, .. } if program.is_empty() => {
                return Err("empty executable".into());
            }
            PluginProvider::WebSocket { url } if !url.starts_with("ws://") => {
                return Err("plugin WebSocket transport requires ws://".into());
            }
            _ => {}
        }
        Ok(())
    }
    pub fn to_value(&self) -> Value {
        let mut object = Object::new();
        object.insert("id", self.id.clone());
        object.insert("revision", self.revision.clone());
        object.insert("enabled", self.enabled);
        object.insert("generation", self.generation);
        object.insert("configuration", self.configuration.clone());
        let mut source_bindings = Object::new();
        for (operation, source) in &self.source_bindings {
            source_bindings.insert(operation.clone(), source.clone());
        }
        object.insert("source_bindings", Value::Object(source_bindings));
        if let Some(schema) = &self.configuration_schema {
            let mut encoded = Object::new();
            encoded.insert(
                "type",
                facet_json::to_string(&schema.ty).expect("schema serialization"),
            );
            let mut definitions = Object::new();
            for (name, definition) in &schema.definitions {
                definitions.insert(
                    name.clone(),
                    facet_json::to_string(definition).expect("schema serialization"),
                );
            }
            encoded.insert("definitions", Value::Object(definitions));
            object.insert("configuration_schema", Value::Object(encoded));
        }
        object.insert(
            "exports",
            Value::List(
                self.exports
                    .iter()
                    .map(|export| {
                        let mut value = Object::new();
                        for (key, value_text) in [
                            ("export", &export.export),
                            ("package", &export.package),
                            ("module", &export.module),
                            ("interface", &export.interface),
                            ("package_version", &export.package_version),
                            ("fingerprint", &export.fingerprint),
                        ] {
                            value.insert(key, value_text.clone());
                        }
                        value.insert(
                            "contract",
                            export
                                .contract
                                .clone()
                                .map(Value::String)
                                .unwrap_or(Value::Null),
                        );
                        Value::Object(value)
                    })
                    .collect(),
            ),
        );
        object.insert(
            "priority",
            self.priority
                .map(|v| Value::I64(v as i64))
                .unwrap_or(Value::Null),
        );
        let mut provider = Object::new();
        provider.insert("kind", self.provider.kind().to_string());
        match &self.provider {
            PluginProvider::Rust { key } => {
                provider.insert("key", key.clone());
            }
            PluginProvider::WebSocket { url } => {
                provider.insert("url", url.clone());
            }
            PluginProvider::Stdio {
                program,
                args,
                cwd,
                env,
            } => {
                provider.insert("program", program.clone());
                provider.insert(
                    "args",
                    Value::List(args.iter().cloned().map(Value::String).collect()),
                );
                provider.insert("cwd", cwd.clone().map(Value::String).unwrap_or(Value::Null));
                let mut values = Object::new();
                for (key, value) in env {
                    values.insert(key.clone(), value.clone());
                }
                provider.insert("env", Value::Object(values));
            }
        }
        object.insert("provider", Value::Object(provider));
        Value::Object(object)
    }
    pub fn from_value(value: &Value) -> Result<Self, String> {
        fn string(object: &Object, name: &str) -> Result<String, String> {
            object
                .get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("invalid {name}"))
        }
        let Value::Object(object) = value else {
            return Err("expected activation object".into());
        };
        let Some(Value::Object(provider)) = object.get("provider") else {
            return Err("expected provider".into());
        };
        let provider = match string(provider, "kind")?.as_str() {
            "rust" => PluginProvider::Rust {
                key: string(provider, "key")?,
            },
            "websocket" => PluginProvider::WebSocket {
                url: string(provider, "url")?,
            },
            "stdio" => PluginProvider::Stdio {
                program: string(provider, "program")?,
                args: match provider.get("args") {
                    None => Vec::new(),
                    Some(Value::List(v)) => v
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| "invalid argument".into())
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                    _ => return Err("invalid args".into()),
                },
                cwd: match provider.get("cwd") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(v)) => Some(v.clone()),
                    _ => return Err("invalid cwd".into()),
                },
                env: match provider.get("env") {
                    None => BTreeMap::new(),
                    Some(Value::Object(v)) => v
                        .iter()
                        .map(|(k, v)| {
                            v.as_str()
                                .map(|v| (k.clone(), v.to_owned()))
                                .ok_or_else(|| "invalid env".into())
                        })
                        .collect::<Result<BTreeMap<_, _>, String>>()?,
                    _ => return Err("invalid env".into()),
                },
            },
            _ => return Err("unknown plugin provider".into()),
        };
        let activation = Self {
            id: string(object, "id")?,
            revision: string(object, "revision")?,
            provider,
            enabled: match object.get("enabled") {
                Some(Value::Bool(v)) => *v,
                _ => return Err("invalid enabled".into()),
            },
            generation: match object.get("generation") {
                Some(Value::U64(v)) => *v,
                _ => return Err("invalid generation".into()),
            },
            configuration: object.get("configuration").cloned().unwrap_or(Value::Null),
            source_bindings: match object.get("source_bindings") {
                None => BTreeMap::new(),
                Some(Value::Object(bindings)) => bindings
                    .iter()
                    .map(|(operation, source)| {
                        source
                            .as_str()
                            .map(|source| (operation.clone(), source.to_owned()))
                            .ok_or_else(|| "invalid source binding".to_owned())
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?,
                _ => return Err("invalid source bindings".into()),
            },
            configuration_schema: match object.get("configuration_schema") {
                None | Some(Value::Null) => None,
                Some(Value::Object(schema)) => {
                    let ty = facet_json::from_str(&string(schema, "type")?)
                        .map_err(|e| e.to_string())?;
                    let Some(Value::Object(definitions)) = schema.get("definitions") else {
                        return Err("invalid configuration schema definitions".into());
                    };
                    let definitions = definitions
                        .iter()
                        .map(|(name, value)| {
                            let value = value.as_str().ok_or("invalid configuration definition")?;
                            facet_json::from_str(value)
                                .map(|definition| (name.clone(), definition))
                                .map_err(|e| e.to_string())
                        })
                        .collect::<Result<BTreeMap<_, _>, String>>()?;
                    Some(PluginConfigurationSchema { ty, definitions })
                }
                _ => return Err("invalid configuration schema".into()),
            },
            exports: match object.get("exports") {
                None => Vec::new(),
                Some(Value::List(exports)) => exports
                    .iter()
                    .map(|export| {
                        let Value::Object(export) = export else {
                            return Err("invalid export".into());
                        };
                        Ok(PluginExport {
                            export: string(export, "export")?,
                            package: string(export, "package")?,
                            module: string(export, "module")?,
                            interface: string(export, "interface")?,
                            package_version: string(export, "package_version")?,
                            fingerprint: string(export, "fingerprint")?,
                            contract: match export.get("contract") {
                                None | Some(Value::Null) => None,
                                Some(Value::String(v)) => Some(v.clone()),
                                _ => return Err("invalid contract".into()),
                            },
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                _ => return Err("invalid exports".into()),
            },
            priority: match object.get("priority") {
                None | Some(Value::Null) => None,
                Some(Value::I32(v)) => Some(*v),
                Some(Value::U64(v)) => {
                    Some(i32::try_from(*v).map_err(|_| "priority out of range")?)
                }
                Some(Value::I64(v)) => {
                    Some(i32::try_from(*v).map_err(|_| "priority out of range")?)
                }
                _ => return Err("invalid priority".into()),
            },
        };
        activation.validate()?;
        Ok(activation)
    }
}

pub fn package() -> Package {
    let attribute = AttributeType {
        id: DESCRIPTOR_ATTR.into(),
        name: "descriptor".into(),
        ty: Type::new(TypeKind::Record(RecordType {
            fields: BTreeMap::new(),
            open: true,
            additional: None,
            required_order: None,
        })),
        constraints: vec![],
        meta: Meta::default(),
    };
    let class = ClassType {
        id: CLASS_ID.into(),
        name: "PluginActivation".into(),
        inherits: None,
        extends: vec![],
        strict_schema: true,
        creatable_in_ui: Some(false),
        attributes: BTreeMap::from([(
            "descriptor".into(),
            ClassAttribute {
                attribute: AttributeRef {
                    id: DESCRIPTOR_ATTR.into(),
                },
                required: true,
                ui_order: None,
                computed: None,
                constraints: vec![],
                meta: Meta::default(),
            },
        )]),
        constraints: vec![],
        meta: Meta::default(),
    };
    Package {
        name: PACKAGE_NAME.into(),
        root: Module {
            name: "v1".into(),
            constants: BTreeMap::new(),
            types: BTreeMap::new(),
            attributes: BTreeMap::from([(DESCRIPTOR_ATTR.into(), attribute.clone())]),
            classes: BTreeMap::from([(CLASS_ID.into(), class.clone())]),
            interfaces: BTreeMap::new(),
            contracts: BTreeMap::new(),
            meta: Meta::default(),
        },
        modules: BTreeMap::new(),
        migrations: vec![Migration {
            module: "v1".into(),
            name: "001_init".into(),
            description: Some("Scope plugin installations".into()),
            operations: vec![
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertAttribute { attribute }),
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertClass { class }),
                MigrationOperation::Ddl(MigrationDdlOperation::UpsertCollection {
                    name: COLLECTION.into(),
                    kind: MigrationCollectionKind::Polymorphic,
                    integrity_mode: MigrationIntegrityMode::StrictRegisteredSchema,
                }),
            ],
            meta: Meta::default(),
        }],
        version: None,
        meta: Meta::default(),
    }
}
